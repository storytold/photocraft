//! What the develop view shows while a mask brush paints, frame by frame: the full-resolution
//! frame on screen stays (never the proxy stretched, never a draft, no busy indicator); with the
//! overlay shown only the overlay follows the brush, with it hidden the photo is patched where the
//! brush went; after the stroke the render of its settings replaces it without a softer frame in
//! between.
//!
//! `PHOTOCRAFT_LC_SNAPSHOTS=<dir>` saves `live-stroke.png` there (on the GPU): an Auto Mask stroke
//! held with the overlay hidden over an Exposure of +1, the photo patched where it went; and
//! `overlay-<mode>.png`, the mask in each overlay mode;
//! `PHOTOCRAFT_LC_RAW=<file>` uses that raw.
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, Services, camera_raw_ui, theme::ThemeKind};
use serde_json::{Value, json};

/// A raw larger than the proxy, so the preview shows the full-resolution render.
fn raw() -> (String, Vec<u8>) {
    if let Ok(path) = std::env::var("PHOTOCRAFT_LC_RAW") {
        return ("test.CR2".to_string(), std::fs::read(path).unwrap());
    }
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (w, h) = (1600, 1066);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    ("IMG_0001.dng".to_string(), spec.build())
}

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

fn settle(h: &mut Harness<'_, PhotocraftApp>) {
    for _ in 0..1500 {
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

fn button(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

fn dialog(gpu: bool) -> Harness<'static, PhotocraftApp> {
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
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.open_bytes(&name, &bytes).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    assert_eq!(inspect(&h)["panel"], "develop");
    h
}

/// A stroke across the preview held for `moves` frames (each a little apart, as a hand paints);
/// each frame's `live` (what the preview drew), and the frames after the last move until the
/// photo or overlay shows it (`photo`: the photo; else the overlay). The button stays down.
fn stroke(h: &mut Harness<'_, PhotocraftApp>, moves: usize, photo: bool) -> (Vec<Value>, Pos2) {
    let r = &inspect(h)["viewportRect"];
    let f = |i: usize| r[i].as_f64().unwrap() as f32;
    let view = egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3)));
    let at = |k: usize| view.lerp_inside(vec2(0.3 + 0.3 * k as f32 / moves as f32, 0.45));
    h.event(Event::PointerMoved(at(0)));
    h.run_steps(1);
    button(h, at(0), true);
    let mut frames = Vec::new();
    for k in 1..=moves {
        h.event(Event::PointerMoved(at(k)));
        h.run_steps(1);
        frames.push(inspect(h)["live"].clone());
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    // the last move's pixels: patched in (or the overlay made) while the button is still down
    for _ in 0..400 {
        h.run_steps(1);
        let live = inspect(h)["live"].clone();
        frames.push(live.clone());
        let shown = if photo { &live["photoSeq"] } else { &live["overlaySeq"] };
        if shown == &live["editSeq"] {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (frames, at(moves))
}

/// No frame of a stroke shows the proxy enlarged, a draft or the busy indicator, with the overlay
/// shown or hidden (an effect on the mask), and the stroke's last move reaches the screen while the
/// button is held: the overlay with it shown, the photo (patched) with it hidden. After the stroke
/// the render of its settings replaces the frame, never the proxy in between.
#[test]
fn a_stroke_never_shows_the_proxy_or_a_draft() {
    let gpu = std::env::var_os("PHOTOCRAFT_LC_SNAPSHOTS").is_some();
    let _gpu = gpu.then(photocraft_testkit::gpu_lock);
    let mut h = dialog(gpu);
    ui(&mut h, json!({"local": {"tool": "masking", "newMask": "brush"}}));
    ui(&mut h, json!({"local": {"brush": {"autoMask": true, "size": 0.04, "points": [[0.3, 0.3], [0.4, 0.3]]}}}));
    assert_eq!(inspect(&h)["live"]["shown"], "window", "the full-resolution render shows before the stroke: {}", inspect(&h)["live"]);
    for hidden in [false, true] {
        if hidden {
            ui(&mut h, json!({"local": {"adjust": {"exposure": 1.0}}}));
            assert_eq!(inspect(&h)["maskOverlay"], false);
        }
        let revision = inspect(&h)["previewRevision"].clone();
        let (frames, end) = stroke(&mut h, 12, hidden);
        for (i, live) in frames.iter().enumerate() {
            assert_eq!(live["shown"], "window", "frame {i} (overlay hidden: {hidden}): {live}");
            assert_eq!(live["spinner"], false, "frame {i}: no busy indicator while painting");
        }
        let last = frames.last().unwrap();
        let shown = if hidden { &last["photoSeq"] } else { &last["overlaySeq"] };
        assert_eq!(shown, &last["editSeq"], "the stroke's last move is on screen before the release (overlay hidden: {hidden}): {last}");
        assert_eq!(inspect(&h)["previewRevision"], revision, "the proxy develops nothing while the stroke goes on");
        if hidden && gpu {
            let dir = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_LC_SNAPSHOTS").unwrap());
            let _ = std::fs::create_dir_all(&dir);
            h.render().unwrap().save(dir.join("live-stroke.png")).unwrap();
        }
        // let go: frame by frame until the render of the stroke's settings is in
        button(&mut h, end, false);
        for i in 0..1500 {
            h.run_steps(1);
            let r = inspect(&h);
            assert_eq!(r["live"]["shown"], "window", "frame {i} after the stroke: {}", r["live"]);
            if r["previewPending"] == false && r["detailPending"] == false && r["previewRevision"] != revision {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        settle(&mut h);
        let r = inspect(&h);
        assert_eq!((r["live"]["shown"].as_str(), r["previewApproximate"].as_bool()), (Some("window"), Some(false)), "settled: {}", r["live"]);
        assert_eq!(r["live"]["photoSeq"], r["live"]["editSeq"]);
    }
    // each overlay mode drawn over the frame on screen: nothing renders again
    let revision = inspect(&h)["previewRevision"].clone();
    for mode in ["color", "colorOnBw", "imageOnBlack", "imageOnWhite", "whiteOnBlack"] {
        ui(&mut h, json!({"local": {"overlay": true, "overlayMode": mode}}));
        assert_eq!(inspect(&h)["live"]["shown"], "window");
        if gpu {
            let dir = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_LC_SNAPSHOTS").unwrap());
            h.render().unwrap().save(dir.join(format!("overlay-{mode}.png"))).unwrap();
        }
    }
    assert_eq!(inspect(&h)["previewRevision"], revision, "an overlay mode develops nothing");
}
