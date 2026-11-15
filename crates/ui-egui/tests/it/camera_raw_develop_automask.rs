//! Auto Mask on a real photo, looked at: `PHOTOCRAFT_LC_CAKE=<raw>` paints scripted Auto Mask
//! strokes on it in the develop view (GPU harness, the dialog's preview size) and saves overlay
//! snapshots into `PHOTOCRAFT_LC_SNAPSHOTS` (`<prefix>-<case>-overlay.png`, and
//! `<prefix>-<case>-exposure.png` with +1.5 exposure on the mask and the overlay hidden;
//! `PHOTOCRAFT_LC_PREFIX`, default `cake`), and `<prefix>-<case>-develop.json`, the develop
//! settings with the strokes as painted (craft-develop's `photo_probe` takes them to reproduce
//! the dialog's render). `PHOTOCRAFT_LC_BRUSH` sets the brush size. Without the raw the test does
//! nothing.
//!
//! The strokes are placed for a photo of a white tiered cake on a gold board in front of a grey
//! wall, the board's dark shadow on the wall below and right of it: `PHOTOCRAFT_LC_STROKES` (a JSON
//! object of case name to a list of strokes, each a list of `[x, y]` fractions of the preview)
//! replaces them.
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, Services, camera_raw_ui, theme::ThemeKind};
use serde_json::{Value, json};

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

fn settle(h: &mut Harness<'_, PhotocraftApp>) {
    for _ in 0..1200 {
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

fn preview_rect(h: &Harness<'_, PhotocraftApp>) -> egui::Rect {
    let r = &inspect(h)["previewRect"];
    let f = |i: usize| r[i].as_f64().unwrap() as f32;
    egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3)))
}

/// One stroke through `pts` (fractions of the preview), a few pointer moves per segment.
fn stroke(h: &mut Harness<'_, PhotocraftApp>, pts: &[egui::Vec2]) {
    let r = preview_rect(h);
    let Some(first) = pts.first() else { return };
    h.event(Event::PointerMoved(r.lerp_inside(*first)));
    h.run_steps(1);
    button(h, r.lerp_inside(*first), true);
    h.run_steps(1);
    for w in pts.windows(2) {
        for k in 1..=6 {
            h.event(Event::PointerMoved(r.lerp_inside(w[0] + (w[1] - w[0]) * (k as f32 / 6.0))));
            h.run_steps(1);
        }
    }
    button(h, r.lerp_inside(*pts.last().unwrap_or(first)), false);
    settle(h);
}

fn default_cases() -> Value {
    json!({
        // the centre on the white fondant of the lower tier, near its right edge and the board
        "cake": [[[0.5082, 0.5405], [0.5246, 0.5467], [0.5328, 0.559], [0.5369, 0.5774], [0.5287, 0.5897], [0.5123, 0.5958], [0.5, 0.5897], [0.4918, 0.5774], [0.5041, 0.5651], [0.5246, 0.5713]]],
        // down the lower tier's right side, the brush reaching over the wall and the board
        "edge": [[[0.5246, 0.5405], [0.5344, 0.5528], [0.5393, 0.5688], [0.5418, 0.5848], [0.5377, 0.5995], [0.5246, 0.6044]]],
        // the centre on the board's shadow on the wall
        "shadow": [[[0.4754, 0.6634], [0.5, 0.6695], [0.5246, 0.6609], [0.541, 0.6511], [0.5123, 0.6757]]]
    })
}

#[test]
fn auto_mask_on_a_real_photo() {
    let Ok(path) = std::env::var("PHOTOCRAFT_LC_CAKE") else { return };
    let dir = std::env::var("PHOTOCRAFT_LC_SNAPSHOTS").unwrap_or_else(|_| std::env::temp_dir().join("photocraft-automask").display().to_string());
    let prefix = std::env::var("PHOTOCRAFT_LC_PREFIX").unwrap_or_else(|_| "cake".into());
    let cases: Value = std::env::var("PHOTOCRAFT_LC_STROKES").ok().map(|s| serde_json::from_str(&s).unwrap()).unwrap_or_else(default_cases);
    let size: Option<f64> = std::env::var("PHOTOCRAFT_LC_BRUSH").ok().and_then(|s| s.parse().ok());
    std::fs::create_dir_all(&dir).unwrap();
    let _lock = photocraft_testkit::gpu_lock();
    let bytes = std::fs::read(&path).unwrap();
    let name = std::path::Path::new(&path).file_name().unwrap().to_string_lossy().to_string();
    let save = |h: &mut Harness<'_, PhotocraftApp>, n: &str| {
        h.render().unwrap().save(std::path::Path::new(&dir).join(format!("{prefix}-{n}.png"))).unwrap();
    };
    for (case, strokes) in cases.as_object().unwrap() {
        let (name, bytes) = (name.clone(), bytes.clone());
        let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1600.0, 1000.0)).wgpu().build_eframe(move |cc| {
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
        assert_eq!(inspect(&h)["panel"], "develop");
        if case == "plain" {
            save(&mut h, "plain");
            continue;
        }
        key(&mut h, Key::K);
        if let Some(s) = size {
            ui(&mut h, json!({"local": {"brush": {"size": s}}}));
        }
        assert_eq!(inspect(&h)["brush"]["autoMask"], true);
        for s in strokes.as_array().unwrap() {
            let pts: Vec<egui::Vec2> = s.as_array().unwrap().iter().map(|p| vec2(p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32)).collect();
            stroke(&mut h, &pts);
        }
        save(&mut h, &format!("{case}-overlay"));
        let r = inspect(&h);
        std::fs::write(std::path::Path::new(&dir).join(format!("{prefix}-{case}-develop.json")), r["develop"].to_string()).unwrap();
        let mut masks = r["develop"]["masks"].clone();
        masks[0]["adjust"]["exposure"] = json!(1.5);
        ui(&mut h, json!({"develop": {"masks": masks}, "local": {"overlay": false}}));
        save(&mut h, &format!("{case}-exposure"));
    }
}
