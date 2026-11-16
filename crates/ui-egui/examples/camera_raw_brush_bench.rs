//! What the develop view shows while a mask brush paints, frame by frame, on a real raw:
//! `cargo run --release -p photocraft-ui-egui --example camera_raw_brush_bench -- <raw>` opens it in
//! the real shell (egui_kittest, no window) at 2 pixels per point and paints a 60-move stroke at
//! 60 Hz at fit, 100 % and 136 %, with Auto Mask on and off, with the mask's overlay shown and
//! with it hidden over an Exposure of +1. For each it prints how many frames showed the stretched
//! proxy or a draft (softer than the screen), how many drew the busy indicator, the UI frame time
//! (median and longest), and the time from a pointer move to pixels on screen that show it (the
//! overlay when it shows, the photo when not). `-v` prints every frame. CPU work only: the
//! textures aren't presented.
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, Services, camera_raw_ui, theme::ThemeKind};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

fn menu(h: &mut Harness<'_, PhotocraftApp>, ui: Value) -> Value {
    let ctx = h.ctx.clone();
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({ "ui": ui })).unwrap().unwrap()
}

fn idle(r: &Value) -> bool {
    !r.is_null() && r["previewPending"] == false && r["detailPending"] == false && r["previewDraft"] != true
}

fn settle(h: &mut Harness<'_, PhotocraftApp>) -> Duration {
    let t = Instant::now();
    let mut quiet = 0;
    for _ in 0..3000 {
        h.run_steps(1);
        quiet = if idle(&inspect(h)) { quiet + 1 } else { 0 };
        if quiet >= 3 {
            return t.elapsed();
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    t.elapsed()
}

fn button(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

struct Stroke {
    frames: usize,
    soft: usize,
    spinner: usize,
    frame_med: f64,
    frame_max: f64,
    lat_med: f64,
    lat_max: f64,
    unanswered: usize,
    settle: f64,
    end_shown: String,
}

/// A stroke of `moves` pointer moves across `view` at 60 Hz; `photo`: the photo (not the
/// overlay) is what shows the mask.
fn stroke(h: &mut Harness<'_, PhotocraftApp>, view: egui::Rect, moves: usize, photo: bool, verbose: bool) -> Stroke {
    let at = |k: usize| {
        let t = k as f32 / moves.max(1) as f32;
        view.lerp_inside(vec2(0.3 + 0.4 * t, 0.45 + 0.1 * (t * 9.0).sin()))
    };
    h.event(Event::PointerMoved(at(0)));
    h.run_steps(1);
    button(h, at(0), true);
    let mut frames = Vec::new();
    let mut sent: Vec<(u64, Instant)> = Vec::new();
    let mut answered: Vec<Duration> = Vec::new();
    let (mut soft, mut spinner) = (0, 0);
    let mut k = 0;
    // the moves, then frames until the last one's pixels are in (or a second)
    let start = Instant::now();
    loop {
        let frame = Instant::now();
        if k < moves {
            k += 1;
            h.event(Event::PointerMoved(at(k)));
        }
        let t = Instant::now();
        h.run_steps(1);
        frames.push(ms(t.elapsed()));
        let r = inspect(h);
        let live = &r["live"];
        let edit = live["editSeq"].as_u64().unwrap_or(0);
        if sent.last().is_none_or(|(e, _)| *e < edit) {
            sent.push((edit, frame));
        }
        let shown_seq = if photo { live["photoSeq"].as_u64() } else { live["overlaySeq"].as_u64() }.unwrap_or(0);
        while let Some((e, when)) = sent.first().copied() {
            if e > shown_seq {
                break;
            }
            answered.push(when.elapsed());
            sent.remove(0);
        }
        let shown = live["shown"].as_str().unwrap_or("").to_string();
        if shown == "proxyEnlarged" || shown == "windowDraft" {
            soft += 1;
        }
        if live["spinner"] == true {
            spinner += 1;
        }
        if verbose {
            println!(
                "  frame {:3} {:6.1} ms shown {:13} spinner {:5} edit {:3} photo {:3} overlay {:3} maskOverlay {} patch {} ms {} px",
                frames.len(),
                frames.last().copied().unwrap_or(0.0),
                shown,
                live["spinner"],
                edit,
                live["photoSeq"],
                live["overlaySeq"],
                r["maskOverlay"],
                live["patchMs"],
                live["patchPixels"]
            );
        }
        if k >= moves && (sent.is_empty() || start.elapsed() > Duration::from_secs(4)) {
            break;
        }
        let spent = frame.elapsed();
        if spent < Duration::from_micros(16_667) {
            std::thread::sleep(Duration::from_micros(16_667) - spent);
        }
    }
    let unanswered = sent.len();
    button(h, at(moves), false);
    let settle = ms(settle(h));
    let end_shown = inspect(h)["live"]["shown"].as_str().unwrap_or("").to_string();
    let mut f = frames.clone();
    f.sort_by(f64::total_cmp);
    let mut l: Vec<f64> = answered.iter().map(|d| ms(*d)).collect();
    l.sort_by(f64::total_cmp);
    Stroke {
        frames: frames.len(),
        soft,
        spinner,
        frame_med: f.get(f.len() / 2).copied().unwrap_or(0.0),
        frame_max: f.last().copied().unwrap_or(0.0),
        lat_med: l.get(l.len() / 2).copied().unwrap_or(0.0),
        lat_max: l.last().copied().unwrap_or(0.0),
        unanswered,
        settle,
        end_shown,
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: camera_raw_brush_bench <raw file> [-v]");
        return;
    };
    let rest: Vec<String> = args.collect();
    let verbose = rest.iter().any(|a| a == "-v");
    let bytes = std::fs::read(&path).unwrap();
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1500.0, 950.0)).with_pixels_per_point(2.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.open_bytes("bench.CR2", &bytes).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    let r = inspect(&h);
    assert_eq!(r["panel"], "develop");
    println!("source {} proxy {} at 2x", r["sourceSize"], r["proxy"]);
    menu(&mut h, json!({"local": {"tool": "masking"}}));
    settle(&mut h);
    let mut rows = Vec::new();
    let only = rest.iter().find_map(|a| a.strip_prefix("--only=")).map(str::to_string);
    for (label, zoom) in [("fit", json!("fit")), ("100%", json!(1.0)), ("136%", json!(1.36))] {
        if only.as_ref().is_some_and(|o| o != label) {
            continue;
        }
        for auto in [true, false] {
            for hidden in [false, true] {
                menu(&mut h, json!({"view": {"zoom": zoom, "center": [0.5, 0.5]}}));
                // a mask with a stroke on it, and (hidden) an effect: Exposure +1 hides the overlay
                menu(&mut h, json!({"local": {"newMask": "brush", "brush": {"autoMask": auto, "size": 0.03, "points": [[0.4, 0.3], [0.5, 0.32]]}}}));
                if hidden {
                    menu(&mut h, json!({"local": {"adjust": {"exposure": 1.0}}}));
                }
                settle(&mut h);
                let r = inspect(&h);
                let v = &r["viewportRect"];
                let view = egui::Rect::from_min_max(
                    egui::pos2(v[0].as_f64().unwrap() as f32, v[1].as_f64().unwrap() as f32),
                    egui::pos2(v[2].as_f64().unwrap() as f32, v[3].as_f64().unwrap() as f32),
                );
                let name = format!("{label} auto {} {}", if auto { "on " } else { "off" }, if hidden { "hidden+exp" } else { "overlay   " });
                if verbose {
                    println!("{name}: before the stroke shown {}", r["live"]["shown"]);
                }
                let s = stroke(&mut h, view, 60, hidden, verbose);
                rows.push(format!(
                    "| {name} | {} | {} | {} | {:.1} | {:.1} | {:.0} | {:.0} | {} | {:.0} | {} |",
                    s.frames, s.soft, s.spinner, s.frame_med, s.frame_max, s.lat_med, s.lat_max, s.unanswered, s.settle, s.end_shown
                ));
                // the mask goes, so the next stroke starts from the same settings
                for pressed in [true, false] {
                    h.event(Event::Key { key: Key::Delete, physical_key: Some(Key::Delete), pressed, repeat: false, modifiers: Modifiers::NONE });
                }
                settle(&mut h);
            }
        }
    }
    println!(
        "| stroke | frames | soft frames (proxy/draft) | spinner frames | UI frame med ms | UI frame max ms | move→pixels med ms | move→pixels max ms | moves never shown | settle ms | shown after |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for row in rows {
        println!("{row}");
    }
}
