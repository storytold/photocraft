//! The develop view's vertical rhythm: a slider row (label and value over its track) is about 31
//! points, in the Edit panel, a mask's effects and the brush's controls alike, with the knob and the
//! value still comfortably hit.
//!
//! `PHOTOCRAFT_LC_SNAPSHOTS=<dir>` saves `density.png` there (on the GPU, at 2 pixels per point):
//! the Edit panel with Light and Color open beside the Masking brush; `PHOTOCRAFT_LC_RAW=<file>`
//! uses that raw.
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

fn ui(h: &mut Harness<'_, PhotocraftApp>, v: Value) {
    let ctx = h.ctx.clone();
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({ "ui": v })).unwrap().unwrap();
    settle(h);
}

fn slider(h: &Harness<'_, PhotocraftApp>, label: &str) -> egui::Rect {
    h.get_by_role_and_label(egui::accesskit::Role::Slider, label).rect()
}

#[test]
fn slider_rows_are_about_thirty_one_points() {
    let gpu = std::env::var_os("PHOTOCRAFT_LC_SNAPSHOTS").is_some();
    let _lock = gpu.then(photocraft_testkit::gpu_lock);
    let (name, bytes) = match std::env::var("PHOTOCRAFT_LC_RAW") {
        Ok(path) => ("test.CR2".to_string(), std::fs::read(path).unwrap()),
        Err(_) => ("IMG_0001.dng".to_string(), dng()),
    };
    let builder = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1400.0, 1000.0)).with_pixels_per_point(2.0);
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
    ui(&mut h, json!({"local": {"sections": ["light", "color"]}}));
    if let Ok(dir) = std::env::var("PHOTOCRAFT_LC_SNAPSHOTS") {
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(std::path::Path::new(&dir).join("density.png")).unwrap();
    }
    let pitch = |h: &Harness<'_, PhotocraftApp>, a: &str, b: &str| slider(h, b).center().y - slider(h, a).center().y;
    let (p1, p2) = (pitch(&h, "Exposure", "Contrast"), pitch(&h, "Highlights", "Shadows"));
    // the knob's hit area stays at least as tall as the knob
    let knob = slider(&h, "Exposure").height();
    let mask_pitch = |h: &mut Harness<'_, PhotocraftApp>| {
        ui(h, json!({"local": {"tool": "masking", "newMask": "brush"}}));
        let p = pitch(h, "Size", "Feather");
        if let Ok(dir) = std::env::var("PHOTOCRAFT_LC_SNAPSHOTS") {
            h.render().unwrap().save(std::path::Path::new(&dir).join("density-mask.png")).unwrap();
        }
        p
    };
    let p3 = mask_pitch(&mut h);
    eprintln!("slider pitch: Edit {p1} / {p2}, brush {p3} points");
    assert!((29.0..=32.0).contains(&p1) && (p1 - p2).abs() < 0.5, "a slider row is {p1} points ({p2})");
    assert!(knob >= 16.0, "the knob is hit {knob} points tall");
    // a mask's effects and the brush's controls keep the same rhythm
    assert!((p3 - p1).abs() < 0.5, "the brush's rows are {p3} points");
}
