//! Free Transform stalls (#11103): two selected raster layers on a 24 MP document, one box.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example free_transform_bench -- [--size 6000x4000] [--reps 9] [--steps 60]
//! ```
//!
//! - begin: Ctrl+T (`transform_tool::begin`) and the first frame of the document area, then Esc.
//! - drag update: one pointer move and the frame of the document area it runs. "scale" drags the
//!   bottom-right handle towards the top-left corner, shrinking the box from 100 % to 20 % at 100 %
//!   view zoom, so the scale crosses ½ and ¼ (new mip levels) mid-drag; "move" drags the box by its
//!   middle.
//!
//! CPU side only: the app's document area on the CPU canvas and the texture preparation it does,
//! not GPU upload or presentation. A helper thread's work (a mip level built off the UI thread)
//! isn't counted.

use std::time::Instant;

use craft::behavior::kurbo::Point;
use egui::{Event, Modifiers, PointerButton, Pos2};
use photocraft_ui_egui::{PhotocraftApp, Services, canvas, transform_tool};
use serde_json::json;

type H = egui_kittest::Harness<'static, PhotocraftApp>;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn worst(v: &[f64]) -> f64 {
    v.iter().copied().fold(0.0, f64::max)
}

/// The texture size limit a desktop GPU reports (egui's default without a renderer is 2048,
/// which would shrink the preview to a fraction of the layer and hide the cost being measured).
const MAX_TEXTURE_SIDE: usize = 16384;

/// One frame of the document area, in milliseconds.
fn frame(h: &mut H) -> f64 {
    let start = Instant::now();
    h.step();
    start.elapsed().as_secs_f64() * 1e3
}

/// The screen point of document point `at` through the view the canvas last showed.
fn screen(h: &H, at: [f64; 2]) -> Pos2 {
    canvas::active_view(h.state()).expect("an active document view").to_screen(Point::new(at[0], at[1]))
}

/// Queues the primary button going down or up at document point `at`.
fn button(h: &mut H, at: [f64; 2], pressed: bool) {
    let pos = screen(h, at);
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

/// Queues a pointer move to document point `at`.
fn move_to(h: &mut H, at: [f64; 2]) {
    let pos = screen(h, at);
    h.event(Event::PointerMoved(pos));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (w, h) =
        arg(&args, "--size").and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))).unwrap_or((6000, 4000));
    let reps: usize = arg(&args, "--reps").and_then(|s| s.parse().ok()).unwrap_or(9).max(1);
    let steps: usize = arg(&args, "--steps").and_then(|s| s.parse().ok()).unwrap_or(60).max(2);

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": w, "height": h, "background": "white"})).expect("new document");
    // Two overlapping layers, each covering most of the canvas.
    let (fw, fh) = (w * 2 / 3, h * 3 / 4);
    let mut ids = Vec::new();
    for (name, x, y, color) in [("A", 0, 0, "#c03020"), ("B", w - fw, h - fh, "#2050c0")] {
        let id = app.run("layer.new.layer", json!({"name": name})).expect("new layer")["layer"].as_u64().unwrap_or(0);
        app.run("layer.select", json!({"layer": id})).expect("select");
        app.run("select.rect", json!({"x": x, "y": y, "width": fw, "height": fh})).expect("select.rect");
        app.run("edit.fill", json!({"contents": "color", "color": color})).expect("fill");
        app.run("select.deselect", json!({})).expect("deselect");
        ids.push(id);
    }
    if let (Some(a), Some(b)) = (ids.first(), ids.get(1)) {
        app.run("layer.select", json!({"layer": a})).expect("select A");
        app.run("layer.select", json!({"layer": b, "mode": "add"})).expect("add B");
    }
    app.sync_views();
    if let Some(v) = app.ui.views.first_mut() {
        v.zoom = 1.0;
        v.center = [w as f32 / 2.0, h as f32 / 2.0];
        v.clear_pending();
    }
    let ctx = app.ctx().clone();
    let (doc_w, doc_h) = (w, h);
    let mut h: H = egui_kittest::Harness::builder()
        .with_context(ctx.clone())
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_ui_state(|ui, app: &mut PhotocraftApp| canvas::document_area(app, ui), app);
    h.input_mut().max_texture_side = Some(MAX_TEXTURE_SIDE);
    for _ in 0..4 {
        h.step();
    }

    // Ctrl+T and its first frame.
    let (mut begin, mut first, mut begin_total) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..reps {
        let t0 = Instant::now();
        transform_tool::begin(h.state_mut(), &ctx).expect("Free Transform");
        let opened = t0.elapsed().as_secs_f64() * 1e3;
        let drawn = frame(&mut h);
        begin.push(opened);
        first.push(drawn);
        begin_total.push(opened + drawn);
        transform_tool::cancel(h.state_mut());
        h.step();
    }

    // Drags: one pointer move + its frame per update.
    let mut drag = |kind: &str| -> Vec<f64> {
        transform_tool::begin(h.state_mut(), &ctx).expect("Free Transform");
        h.step();
        let q = h.state().ui.transform.as_ref().map(|t| t.quad_array()).unwrap_or_default();
        let (from, to) = if kind == "scale" {
            // The bottom-right handle towards the top-left corner: 100 % → 20 %.
            let (a, c) = (q[0], q[2]);
            (c, [a[0] + 0.2 * (c[0] - a[0]), a[1] + 0.2 * (c[1] - a[1])])
        } else {
            let m = [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0];
            (m, [m[0] + 400.0, m[1] + 250.0])
        };
        move_to(&mut h, from);
        h.step();
        button(&mut h, from, true);
        h.step();
        let mut times = Vec::with_capacity(steps);
        for s in 1..=steps {
            let k = s as f64 / steps as f64;
            move_to(&mut h, [from[0] + k * (to[0] - from[0]), from[1] + k * (to[1] - from[1])]);
            times.push(frame(&mut h));
        }
        button(&mut h, to, false);
        h.step();
        transform_tool::cancel(h.state_mut());
        h.step();
        times
    };
    let mut scale = Vec::new();
    let mut moving = Vec::new();
    for _ in 0..reps.min(3) {
        scale.extend(drag("scale"));
        moving.extend(drag("move"));
    }

    if let Some(out) = arg(&args, "--json") {
        use photocraft_testkit::perf::{process_peak_rss_bytes, report, row, write_report};
        let rss = photocraft_testkit::perf::current_rss_bytes().max(process_peak_rss_bytes());
        let rows = vec![
            row("Free Transform begin, 2 layers on 24 MP: Ctrl+T + first frame", &begin_total, rss, None),
            row("Free Transform begin, 2 layers on 24 MP: Ctrl+T", &begin, rss, None),
            row("Free Transform begin, 2 layers on 24 MP: first frame", &first, rss, None),
            row("Free Transform drag update, 2 layers on 24 MP: scale through mip levels", &scale, rss, None),
            row("Free Transform drag update, 2 layers on 24 MP: move", &moving, rss, None),
        ];
        let rep = report("free_transform_bench", json!({"width": doc_w, "height": doc_h, "layers": 2, "reps": reps, "steps": steps}), rows, None);
        if let Err(e) = write_report(&out, &rep) {
            eprintln!("{e}");
        }
    }
    println!("{doc_w}×{doc_h}, 2 layers, {reps} reps, {steps} steps per drag");
    println!(
        "begin: Ctrl+T {:.1} ms + first frame {:.1} ms = {:.1} ms (median; worst {:.1} ms)",
        median(&begin),
        median(&first),
        median(&begin_total),
        worst(&begin_total)
    );
    println!("drag update, scale 100 % → 20 %: median {:.2} ms, worst {:.2} ms", median(&scale), worst(&scale));
    println!("drag update, move: median {:.2} ms, worst {:.2} ms", median(&moving), worst(&moving));
}
