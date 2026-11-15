//! How quickly Camera Raw's develop view answers a slider on a real raw:
//! `cargo run --release -p photocraft-ui-egui --example camera_raw_develop_bench -- <raw>` opens it
//! in the real shell (egui_kittest, no window) at 2 pixels per point (a Retina display) and prints,
//! at fit, 100 % and 136 %: the time from a slider move to the preview's next frame and to its
//! settled full-quality frame, a continuous drag's frames per second (proxy and full resolution),
//! how old the settings of each shown frame are, the longest UI frame, and brush painting while
//! zoomed. CPU work only: the textures aren't presented.
use egui::{Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
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

/// Whether nothing is being rendered for the preview any more (whether it then shows the full
/// resolution is `previewApproximate`).
fn idle(r: &Value) -> bool {
    !r.is_null() && r["previewPending"] == false && r["detailPending"] == false && r["previewDraft"] != true
}

/// Frames (as the app runs them, 4 ms apart) until the preview has settled; the time it took.
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
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
}

/// One step, timed.
fn step(h: &mut Harness<'_, PhotocraftApp>) -> Duration {
    let t = Instant::now();
    h.run_steps(1);
    t.elapsed()
}

struct Change {
    first: Duration,
    full: Duration,
    /// Settled on the proxy, not the full resolution.
    approximate: bool,
}

/// A slider moved once (pressed, one move, let go): the time to the next preview frame and to the
/// settled full-quality one.
fn change(h: &mut Harness<'_, PhotocraftApp>, track: egui::Rect, dx: f32) -> Change {
    let at = track.center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    button(h, at, true);
    h.run_steps(1);
    settle(h);
    let revision = inspect(h)["previewRevision"].as_u64().unwrap_or(0);
    let t = Instant::now();
    h.event(egui::Event::PointerMoved(at + vec2(dx, 0.0)));
    button(h, at + vec2(dx, 0.0), false);
    let mut first = None;
    let (mut quiet, mut done) = (0, t.elapsed());
    for _ in 0..3000 {
        h.run_steps(1);
        let r = inspect(h);
        if first.is_none() && r["previewRevision"].as_u64().unwrap_or(0) > revision {
            first = Some(t.elapsed());
        }
        if first.is_some() && idle(&r) {
            if quiet == 0 {
                done = t.elapsed();
            }
            quiet += 1;
            if quiet >= 3 {
                break;
            }
        } else {
            quiet = 0;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let approximate = inspect(h)["previewApproximate"] == true;
    Change { first: first.unwrap_or_default(), full: done, approximate }
}

struct Drag {
    fps: f64,
    full_fps: f64,
    lag: Duration,
    worst_frame: Duration,
    settle: Duration,
    /// The proxy job's median time per frame.
    render_ms: f64,
}

/// A continuous drag of `track` at 60 Hz for `secs`, then let go.
fn drag(h: &mut Harness<'_, PhotocraftApp>, track: egui::Rect, secs: f64, painting: bool) -> Drag {
    let from = if painting { track.lerp_inside(vec2(0.3, 0.4)) } else { track.center() - vec2(track.width() * 0.3, 0.0) };
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    button(h, from, true);
    h.run_steps(1);
    let start = inspect(h);
    let (rev0, refined0) = (start["previewRevision"].as_u64().unwrap_or(0), start["detailRevision"].as_u64().unwrap_or(0));
    let t = Instant::now();
    let mut worst = Duration::ZERO;
    let mut k = 0u32;
    // when each pointer position was sent, to see how far behind the shown frame runs
    let mut sent: Vec<(u64, Instant)> = Vec::new();
    let mut lags = Vec::new();
    let mut renders: Vec<f64> = Vec::new();
    let mut last_rev = rev0;
    while t.elapsed().as_secs_f64() < secs {
        let frame = Instant::now();
        k += 1;
        let p = if painting {
            track.lerp_inside(vec2(0.3 + 0.4 * (k as f32 * 0.01).sin().abs(), 0.4 + 0.2 * (k as f32 * 0.013).sin()))
        } else {
            from + vec2((k as f32 * 0.7) % (track.width() * 0.6), 0.0)
        };
        h.event(egui::Event::PointerMoved(p));
        sent.push((u64::from(k), frame));
        worst = worst.max(step(h));
        let r = inspect(h);
        let rev = r["previewRevision"].as_u64().unwrap_or(0);
        if rev > last_rev {
            // the frame shown answers at most the move sent before the step that started it
            if let Some(age) = r["previewAgeMs"].as_f64() {
                lags.push(Duration::from_secs_f64(age / 1e3));
            }
            renders.push(r["renderMs"].as_f64().unwrap_or(0.0));
            last_rev = rev;
        }
        let spent = frame.elapsed();
        if spent < Duration::from_micros(16_667) {
            std::thread::sleep(Duration::from_micros(16_667) - spent);
        }
    }
    let r = inspect(h);
    let elapsed = t.elapsed().as_secs_f64();
    let fps = (r["previewRevision"].as_u64().unwrap_or(0) - rev0) as f64 / elapsed;
    let full_fps = (r["detailRevision"].as_u64().unwrap_or(0).saturating_sub(refined0)) as f64 / elapsed;
    let end = from + vec2(0.0, 0.0);
    button(h, end, false);
    let settle = settle(h);
    lags.sort();
    let lag = lags.get(lags.len() / 2).copied().unwrap_or_default();
    renders.sort_by(f64::total_cmp);
    let render_ms = renders.get(renders.len() / 2).copied().unwrap_or(0.0);
    Drag { fps, full_fps, lag, worst_frame: worst, settle, render_ms }
}

fn ms(d: Duration) -> String {
    format!("{:.0}", d.as_secs_f64() * 1e3)
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: camera_raw_develop_bench <raw file>");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let bytes2 = bytes.clone();
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
    menu(&mut h, json!({"local": {"sections": ["light", "color"]}}));
    settle(&mut h);
    let mut rows = Vec::new();
    for (label, zoom) in [("fit", json!("fit")), ("100%", json!(1.0)), ("136%", json!(1.36))] {
        menu(&mut h, json!({"view": {"zoom": zoom, "center": [0.45, 0.5]}}));
        settle(&mut h);
        let track = h.get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure").rect();
        let changes: Vec<Change> = (0..5).map(|i| change(&mut h, track, if i % 2 == 0 { 14.0 } else { -14.0 })).collect();
        let first = changes.iter().map(|c| c.first).min().unwrap_or_default();
        let mut fulls: Vec<Duration> = changes.iter().map(|c| c.full).collect();
        fulls.sort();
        let full = fulls.get(fulls.len() / 2).copied().unwrap_or_default();
        let approx = if changes.iter().any(|c| c.approximate) { " (proxy only)" } else { "" };
        let d = drag(&mut h, track, 3.0, false);
        rows.push(format!(
            "| {label} | {} | {}{approx} | {:.1} | {:.1} | {} | {} | {} |",
            ms(first),
            ms(full),
            d.fps,
            d.full_fps,
            ms(d.lag),
            ms(d.worst_frame),
            ms(d.settle)
        ));
    }
    // brush painting while zoomed (136 %)
    menu(&mut h, json!({"local": {"tool": "masking"}}));
    settle(&mut h);
    menu(&mut h, json!({"local": {"newMask": "brush"}}));
    settle(&mut h);
    let rect = egui::Rect::from_min_max(
        egui::pos2(r["viewportRect"][0].as_f64().unwrap() as f32, r["viewportRect"][1].as_f64().unwrap() as f32),
        egui::pos2(r["viewportRect"][2].as_f64().unwrap() as f32, r["viewportRect"][3].as_f64().unwrap() as f32),
    );
    let d = drag(&mut h, rect, 3.0, true);
    rows.push(format!(
        "| brush 136% | | | {:.1} | {:.1} | {} | {} | {} | (proxy job {:.0} ms) |",
        d.fps,
        d.full_fps,
        ms(d.lag),
        ms(d.worst_frame),
        ms(d.settle),
        d.render_ms
    ));
    println!("| view | change → frame ms | change → full ms | drag fps | drag full fps | drag lag ms | worst UI frame ms | settle ms |");
    println!("|---|---|---|---|---|---|---|---|");
    for row in rows {
        println!("{row}");
    }
    let size = |v: &Value| (v[0].as_u64().unwrap_or(1) as usize, v[1].as_u64().unwrap_or(1) as usize);
    engine_timings(&bytes2, size(&r["sourceSize"]), size(&r["proxy"]));
}

/// The engine alone, on this machine's CPU: the proxy, the whole frame at the raw's size (what the
/// zoomed preview rendered before), and windows at the screen's resolution, a tone slider moved
/// between renders (the stage cache keeps the rest).
fn engine_timings(bytes: &[u8], full: (usize, usize), proxy: (usize, usize)) {
    use photocraft_io::raw_develop::{DevelopSource, Quality, ViewWindow};
    let Ok(src) = DevelopSource::new(std::sync::Arc::new(bytes.to_vec()), proxy.0.max(proxy.1), full.0.max(full.1)) else { return };
    let mut s = src.defaults();
    let mut k = 0;
    let mut time = |what: &str, f: &mut dyn FnMut(&craft_develop_model::DevelopSettings)| {
        let mut runs = Vec::new();
        for _ in 0..5 {
            k += 1;
            s.light.exposure = if k % 2 == 0 { 0.3 } else { -0.3 };
            let t = Instant::now();
            f(&s);
            runs.push(t.elapsed());
        }
        runs.sort();
        println!("| {what} | {} | {} |", ms(runs[0]), ms(runs[2]));
    };
    println!("\n| engine render (exposure moved each time) | first ms | median ms |\n|---|---|---|");
    time(&format!("proxy {}×{} full", proxy.0, proxy.1), &mut |s| drop(src.develop_proxy_at(s, proxy.0, proxy.1, Quality::Full)));
    time(&format!("proxy {}×{} draft", proxy.0, proxy.1), &mut |s| drop(src.develop_proxy_at(s, proxy.0, proxy.1, Quality::Draft)));
    time(&format!("whole frame {}×{} (old zoomed preview)", full.0, full.1), &mut |s| drop(src.develop_full_with(s, full.0, full.1)));
    let fit = (2400, (2400 * full.1) / full.0.max(1));
    let whole = ViewWindow { size: fit, x: 0, y: 0, w: fit.0, h: fit.1 };
    time(&format!("fit at 2x: {}×{} full", fit.0, fit.1), &mut |s| drop(src.develop_view(s, whole, Quality::Full)));
    time(&format!("fit at 2x: {}×{} draft", fit.0, fit.1), &mut |s| drop(src.develop_view(s, whole, Quality::Draft)));
    let win = ViewWindow { size: full, x: full.0 / 4, y: full.1 / 4, w: 2240.min(full.0 / 2), h: 1536.min(full.1 / 2) };
    time(&format!("136 %: window {}×{} full", win.w, win.h), &mut |s| drop(src.develop_view(s, win, Quality::Full)));
    time(&format!("136 %: window {}×{} draft", win.w, win.h), &mut |s| drop(src.develop_view(s, win, Quality::Draft)));
}
