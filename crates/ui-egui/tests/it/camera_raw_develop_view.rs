//! The RAW Develop dialog's develop view (a raw the shared engine develops) in the real shell:
//! LightCraft's panel and icon rail, masks with their overlay, heal spots and red eyes, each an
//! edit of the develop settings that undoes and goes into Open.
//!
//! `PHOTOCRAFT_LC_SNAPSHOTS=<dir>` renders the dialog on the GPU and saves its look there (Edit,
//! Edit with sections open, Masking, Heal); `PHOTOCRAFT_LC_RAW=<file>` uses that raw instead of the
//! synthetic DNG.
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
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

/// Frames until the preview has settled (the worker's develop is in).
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

fn local(h: &mut Harness<'_, PhotocraftApp>, v: Value) -> Value {
    let ctx = h.ctx.clone();
    let r = camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"local": v}})).unwrap().unwrap();
    settle(h);
    r
}

fn snapshot(h: &mut Harness<'_, PhotocraftApp>, name: &str) {
    if let Ok(dir) = std::env::var("PHOTOCRAFT_LC_SNAPSHOTS") {
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
    }
}

/// The preview's image rect on screen.
fn preview_rect(h: &Harness<'_, PhotocraftApp>) -> egui::Rect {
    let r = &inspect(h)["previewRect"];
    let f = |i: usize| r[i].as_f64().unwrap() as f32;
    egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3)))
}

fn button(h: &mut Harness<'_, PhotocraftApp>, pos: egui::Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
}

/// A press at `from`, moved in steps to `to`, released there.
fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    button(h, from, true);
    h.run_steps(1);
    for k in 1..=8 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 8.0)));
        h.run_steps(1);
    }
    button(h, to, false);
    settle(h);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    button(h, at, true);
    h.run_steps(1);
    button(h, at, false);
    settle(h);
}

/// Masking: a radial drawn with the pointer, a brush mask painted, the overlay, undo, Before.
fn masking(h: &mut Harness<'_, PhotocraftApp>) {
    // Masking: the rail's button, then a radial from its tile
    h.get_by_label("Masking (M)").click();
    settle(h);
    assert_eq!(inspect(h)["tool"], "masking");
    for absent in ["Subject", "Sky", "Background", "Object", "Describe", "People"] {
        assert!(h.query_by_label(absent).is_none(), "no AI mask is offered: {absent}");
    }
    h.get_by_label("Radial").click();
    settle(h);
    let r = inspect(h);
    assert_eq!(r["develop"]["masks"][0]["components"][0]["shape"]["kind"], "radial", "{}", r["develop"]["masks"]);
    let radial = r["selectedMask"].as_u64().unwrap();
    // a drag on the preview draws the radial anew, centred where it was pressed
    let rect = preview_rect(h);
    let at = rect.lerp_inside(vec2(0.3, 0.4));
    drag(h, at, at + vec2(120.0, 60.0));
    let r = inspect(h);
    let shape = &r["develop"]["masks"][0]["components"][0]["shape"];
    let (cx, cy) = (shape["center"]["x"].as_f64().unwrap(), shape["center"]["y"].as_f64().unwrap());
    assert!((cx - 0.3).abs() < 0.02 && (cy - 0.4).abs() < 0.02, "{shape}");
    assert!(shape["rx"].as_f64().unwrap() > shape["ry"].as_f64().unwrap(), "wider than tall: {shape}");
    // its effect, and a brush mask painted across the frame
    let ctx = h.ctx.clone();
    let mut masks = r["develop"]["masks"].clone();
    masks[0]["adjust"]["exposure"] = json!(1.5);
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"develop": {"masks": masks}}})).unwrap().unwrap();
    local(h, json!({"newMask": "brush"}));
    let r = local(h, json!({"brush": {"points": [[0.1, 0.8], [0.3, 0.75], [0.5, 0.8], [0.7, 0.75]], "size": 0.06}}));
    let strokes = &r["develop"]["masks"][1]["components"][0]["shape"]["strokes"];
    assert_eq!(strokes[0]["points"].as_array().map(Vec::len), Some(4), "{strokes}");
    let brush_y = strokes[0]["points"][0]["y"].as_f64().unwrap();
    assert!((brush_y - 0.8).abs() < 0.02, "the frame's point is the photo's without a crop or warp: {brush_y}");
    // and a stroke painted with the pointer
    let rect = preview_rect(h);
    drag(h, rect.lerp_inside(vec2(0.2, 0.2)), rect.lerp_inside(vec2(0.45, 0.25)));
    let r = inspect(h);
    let painted = &r["develop"]["masks"][1]["components"][0]["shape"]["strokes"][1]["points"];
    assert!(painted.as_array().is_some_and(|p| p.len() >= 3), "{painted}");
    let ctx = h.ctx.clone();
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"undo": true}})).unwrap().unwrap();
    let r = inspect(h);
    let mut masks = r["develop"]["masks"].clone();
    masks[1]["adjust"]["exposure"] = json!(-1.5);
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"develop": {"masks": masks}}})).unwrap().unwrap();
    // the overlay shows the selected mask once the preview renders it
    let r = local(h, json!({"selectMask": radial, "overlay": true}));
    assert_eq!(r["selectedMask"], radial);
    settle(h);
    assert_eq!(inspect(h)["maskOverlayShown"], radial);
    snapshot(h, "masking");
    // undo takes the brush stroke back, then the brush mask
    let undo = |h: &mut Harness<'_, PhotocraftApp>| {
        let ctx = h.ctx.clone();
        camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"undo": true}})).unwrap().unwrap()
    };
    let r = undo(h);
    assert_eq!(r["develop"]["masks"][1]["adjust"]["exposure"], 0.0, "the brush mask's effect");
    let r = undo(h);
    assert_eq!(r["develop"]["masks"][1]["components"][0]["shape"]["strokes"].as_array().map(Vec::len), Some(0));
    let r = undo(h);
    assert_eq!(r["develop"]["masks"].as_array().map(Vec::len), Some(1));
    // the mask survives the Before view
    let r = camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"before": true}})).unwrap().unwrap();
    assert_eq!(r["develop"]["masks"].as_array().map(Vec::len), Some(1));
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"before": false}})).unwrap().unwrap();
}

/// Heal: a spot placed with a click and deleted with Delete, then one placed by the control channel.
fn heal(h: &mut Harness<'_, PhotocraftApp>) {
    // Heal: a spot where a click places it
    h.get_by_label("Heal (Q)").click();
    settle(h);
    // a click on the preview places a spot, Delete takes it away again
    let rect = preview_rect(h);
    click(h, rect.lerp_inside(vec2(0.7, 0.3)));
    let r = inspect(h);
    assert_eq!(r["develop"]["spots"].as_array().map(Vec::len), Some(1), "{}", r["develop"]["spots"]);
    let x = r["develop"]["spots"][0]["points"][0]["x"].as_f64().unwrap();
    assert!((x - 0.7).abs() < 0.02, "{x}");
    h.key_press(egui::Key::Delete);
    settle(h);
    assert_eq!(inspect(h)["develop"]["spots"].as_array().map(Vec::len), Some(0));
    let r = local(h, json!({"heal": {"at": [0.4, 0.45], "mode": "heal"}}));
    assert_eq!(r["develop"]["spots"].as_array().map(Vec::len), Some(1), "{}", r["develop"]["spots"]);
    assert_eq!(r["selectedSpot"], 0);
    assert!(r["develop"]["spots"][0]["source_offset"].is_object(), "a heal spot has its source: {}", r["develop"]["spots"][0]);
    snapshot(h, "heal");
}

#[test]
fn the_develop_view_edits_masks_spots_and_red_eyes() {
    let gpu = std::env::var_os("PHOTOCRAFT_LC_SNAPSHOTS").is_some();
    let _lock = gpu.then(photocraft_testkit::gpu_lock);
    let (name, bytes) = match std::env::var("PHOTOCRAFT_LC_RAW") {
        Ok(path) => ("test.CR2".to_string(), std::fs::read(path).unwrap()),
        Err(_) => ("IMG_0001.dng".to_string(), dng()),
    };
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
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.open_bytes(&name, &bytes).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    let r = inspect(&h);
    assert_eq!(r["panel"], "develop", "{}", r["panel"]);
    assert_eq!(r["tool"], "edit");
    // LightCraft's Edit panel: its header buttons and the rail's tools
    for label in ["Auto", "B&W", "Reset", "Edit (E)", "Heal (Q)", "Masking (M)", "Red Eye"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert!(h.query_by_label("Crop").is_none(), "no rail button for a tool that doesn't work here");
    snapshot(&mut h, "edit-default");
    local(&mut h, json!({"sections": ["light", "color", "curve"]}));
    snapshot(&mut h, "edit-sections");
    local(&mut h, json!({"sections": ["color", "mixer"]}));
    snapshot(&mut h, "edit-color");
    let ctx = h.ctx.clone();
    let grade = json!({"grading": {"shadows": {"hue": 220.0, "sat": 35.0}, "highlights": {"hue": 40.0, "sat": 25.0}}});
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"develop": grade}})).unwrap().unwrap();
    local(&mut h, json!({"sections": ["color", "grading"]}));
    snapshot(&mut h, "edit-grading");

    masking(&mut h);
    heal(&mut h);
    // Red Eye
    h.get_by_label("Red Eye").click();
    settle(&mut h);
    let r = local(&mut h, json!({"redEye": {"at": [0.6, 0.4]}}));
    assert_eq!(r["develop"]["red_eye"].as_array().map(Vec::len), Some(1));
    let ctx = h.ctx.clone();
    assert!(camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"local": {"newMask": "subject"}}})).unwrap().is_err(), "no AI masks");
    assert!(camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"local": {"tool": "crop"}}})).unwrap().is_err());

    // Open develops them all
    let r = camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "{r}");
}
