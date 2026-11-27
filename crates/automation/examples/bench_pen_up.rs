//! How long releasing the pen holds up the UI thread for an overlay Brush stroke
//! (`LiveStroke::begin_overlay`), with the commit's composite on the UI thread (`sync`, as #17082
//! left it) or on the stroke's paint thread (`async`, the canvas's `PendingStroke`).
//!
//! `cargo run --release -p photocraft-automation --features gpu --example bench_pen_up -- <sync|async|cpu-live|gpu-live> <size> <hardness> <dabs> [--pace-ms MS]`
//!
//! The document, stroke and display sink are `bench_live_stroke_gpu`'s: 4928x3280, a white
//! background under a pixel layer of non-uniform content, the zigzag at spacing 0.25. The stroke
//! is painted untimed, then its last step is rendered on the GPU, as the canvas showed it.
//!
//! - `sync`: the release frame runs `LiveStroke::into_commit` (the whole stroke's CPU composite),
//!   `paint.stroke`, then renders what the canvas redraws at release (the relayout damage and the
//!   overlay's bounds). The next frame can start only after all of it.
//! - `async`: the release frame hands the stroke to a thread for `into_commit` and waits at most
//!   `STROKE_COMMIT_WAIT` for it, as `commit_brush_stroke` does; until the commit is ready the
//!   canvas shows the live document, which is already on the GPU, so those frames render nothing.
//!   The frame that finds the commit ready runs `paint.stroke` and renders the release damage.
//!
//! - `cpu-live` and `gpu-live` run the whole stroke as the canvas does (#17133 phase 2): the points
//!   go to a paint thread frame by frame, the UI thread waits at most `STROKE_COMMIT_WAIT` for the
//!   step, renders its damage on the GPU and waits for the GPU. `cpu-live` is the CPU overlay
//!   stroke (`begin_overlay`; the thread rasterises each step and the pen-up is `async`'s). In
//!   `gpu-live` the thread makes only the dabs (`begin_gpu`); the UI thread stamps them into the
//!   compositor's external mask and asks for the coverage tiles that went calm, the thread polls
//!   and feeds them, and at pen-up the thread hands the UI thread the last stamps and the request
//!   for every tile left, waits for it, then makes the commit. The canvas itself (egui) is not
//!   driven: each frame is the step's wait, the stamps and tile requests, the compositor's render
//!   of the step's damage into a display-sized texture and the wait for the GPU.
//!
//! Printed: `release_frame_ms` (the UI thread's time from pen-up to the next frame it can draw),
//! `landing_frame_ms` (async: the frame that shows the committed pixels) and
//! `pen_up_to_committed_ms` (from pen-up until the committed pixels are on the GPU). The live
//! modes add `frame_ms` (the UI thread's time per frame during the stroke: mean, p95, max).
use photocraft_engine::Session;
use photocraft_engine::brush_cmds::{LiveCommit, LiveStroke, LiveStrokeExt};
use photocraft_engine::paint::{StrokePoint, TipGeometry};
use photocraft_geom::Rect;
use photocraft_gpu::{Compositor, GpuDab, MaskReadback};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

const W: i32 = 4928;
const H: i32 = 3280;
/// The canvas's wait for the commit in the release frame (`PhotocraftApp::stroke_commit_wait`).
const STROKE_COMMIT_WAIT: Duration = Duration::from_millis(6);
/// How often the UI thread draws a frame while the commit is pending (a 60 Hz display).
const FRAME: Duration = Duration::from_micros(16_667);

/// Pushes a coverage tile stays untouched before the `gpu-live` stroke reads it back (the canvas's `CALM_PUSHES`).
const CALM_PUSHES: u64 = 4;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// Records a copy of each chunk into a display-sized texture, so the render's output is consumed.
struct Display {
    tex: Option<wgpu::Texture>,
}

impl Display {
    fn sink(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, out: photocraft_gpu::ChunkOut<'_>) {
        let (dw, dh) = (2048u32, 1536u32);
        let tex = self.tex.get_or_insert_with(|| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bench_display"),
                size: wgpu::Extent3d { width: dw, height: dh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: out.texture.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        });
        let size = wgpu::Extent3d { width: out.rect.width().min(dw), height: out.rect.height().min(dh), depth_or_array_layers: 1 };
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: out.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            size,
        );
    }
}

fn wait(device: &wgpu::Device) -> Result<(), Box<dyn std::error::Error>> {
    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None })?;
    Ok(())
}

/// The release frame's tail: commit the stroke with the live pixels, then render what the canvas
/// redraws at release. Returns the time it took and the MB it uploaded.
fn land(
    s: &mut Session,
    commit: photocraft_engine::brush_cmds::LiveCommit,
    params: serde_json::Value,
    overlay: Rect,
    gpu: (&mut Compositor, &wgpu::Device, &wgpu::Queue, &mut Display),
) -> Result<(f64, f64), Box<dyn std::error::Error>> {
    let (comp, device, queue, display) = gpu;
    let t = Instant::now();
    let before = s.active().ok_or("no document")?.doc.clone();
    s.offer_live_commit(commit);
    let result = s.execute("paint.stroke", params)?;
    assert!(result.get("fromLive").is_some(), "the commit did not take the live pixels");
    let doc = s.active().ok_or("no document")?.doc.clone();
    let damage = photocraft_compose::relayout_damage(&before, &doc).union(&overlay);
    let mut mb = 0.0;
    if !damage.is_empty() {
        let stats = comp.render(device, queue, &doc, damage, |e, o| display.sink(device, e, o))?;
        wait(device)?;
        mb = stats.bytes_uploaded as f64 / 1e6;
    }
    Ok((ms(t), mb))
}

/// A dab as the stamping shader takes it.
fn gpu_dab(g: &TipGeometry) -> GpuDab {
    GpuDab {
        rect: g.rect,
        cx: g.cx,
        cy: g.cy,
        ro: g.ro,
        core: g.core,
        shoulder: g.shoulder,
        reach2: g.reach2,
        small_reach2: g.small_reach2,
        cs: g.cs,
        sn: g.sn,
        fx: g.fx,
        fy: g.fy,
        proj: g.proj,
        small: g.small,
        flow: g.flow,
        ceil: g.ceil,
    }
}

/// Coverage tiles asked for, and the epoch they were asked at.
type Asked = (Vec<(i32, i32)>, u64);

/// What the UI thread does for a `gpu-live` stroke: the canvas's `GpuWork`.
enum Work {
    Dabs(Vec<GpuDab>),
    Read(Asked),
}

/// A step the paint thread made: the points it painted, the rectangle it changed, the document it
/// shows (the CPU stroke's; the GPU stroke's is the preview document) and the work for the UI thread.
struct Step {
    points: usize,
    damage: Rect,
    doc: Option<Arc<photocraft_doc::Document>>,
    work: Vec<Work>,
}

/// Paint thread to UI thread.
enum Up {
    Step(Step),
    /// The release: the work left for the UI thread, which the thread waits for.
    Final(Vec<Work>),
    /// The commit and the bounds the stroke showed without its dither.
    Done(Result<(LiveCommit, Rect), String>),
}

/// UI thread to paint thread.
enum Down {
    Points(Vec<StrokePoint>),
    Finish,
    Served(bool),
}

/// The paint thread's read-back, for a `gpu-live` stroke.
#[derive(Clone)]
struct Read {
    readback: MaskReadback,
    device: wgpu::Device,
}

fn feed(live: &mut LiveStroke, read: &Option<Read>, block: bool) {
    if let Some(r) = read
        && (block || r.readback.in_flight() > 0)
    {
        for (key, values, tag) in r.readback.poll(&r.device, block) {
            live.feed_coverage(key, &values, tag);
        }
    }
}

/// The stroke's thread, as the canvas's `PaintThread` is: a step for the points that arrived (all
/// that are queued at once make one), read-backs fed between steps, and at the release the last dabs
/// handed to the UI thread, whose answer the thread waits for before it makes the commit.
fn paint_thread(mut live: LiveStroke, read: Option<Read>, from_ui: Receiver<Down>, to_ui: Sender<Up>) {
    let canvas = live.doc.bounds();
    let mut next: Option<Down> = None;
    loop {
        let message = match next.take() {
            Some(m) => Ok(m),
            None => from_ui.recv_timeout(Duration::from_millis(2)),
        };
        match message {
            Ok(Down::Points(mut points)) => {
                while let Ok(more) = from_ui.try_recv() {
                    match more {
                        Down::Points(m) => points.extend(m),
                        other => {
                            next = Some(other);
                            break;
                        }
                    }
                }
                let step = if read.is_some() {
                    let Ok(dabs) = live.push_dabs(&points) else { return };
                    let dabs: Vec<GpuDab> = dabs.iter().map(gpu_dab).collect();
                    let damage = dabs.iter().fold(Rect::EMPTY, |r, d| r.union(&d.rect)).intersect(&canvas);
                    let (keys, epoch) = live.calm_keys(CALM_PUSHES);
                    let mut work = vec![Work::Dabs(dabs)];
                    if !keys.is_empty() {
                        work.push(Work::Read((keys, epoch)));
                    }
                    Step { points: points.len(), damage, doc: None, work }
                } else {
                    let Ok(damage) = live.push(&points) else { return };
                    Step { points: points.len(), damage, doc: Some(live.doc.clone()), work: Vec::new() }
                };
                if to_ui.send(Up::Step(step)).is_err() {
                    return;
                }
            }
            Ok(Down::Finish) => {
                if read.is_some() {
                    let last: Vec<GpuDab> = live.finish_dabs().iter().map(gpu_dab).collect();
                    let work = vec![Work::Dabs(last), Work::Read(live.unfed_keys())];
                    if to_ui.send(Up::Final(work)).is_err() || !matches!(from_ui.recv(), Ok(Down::Served(true))) {
                        return;
                    }
                    for _ in 0..4 {
                        feed(&mut live, &read, true);
                        if read.as_ref().is_none_or(|r| r.readback.in_flight() == 0) {
                            break;
                        }
                    }
                }
                let bounds = live.overlay_bounds().unwrap_or(Rect::EMPTY);
                let _ = to_ui.send(Up::Done(live.into_commit().map(|c| (c, bounds)).map_err(|e| e.to_string())));
                return;
            }
            Ok(Down::Served(_)) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        feed(&mut live, &read, false);
    }
}

/// The GPU the UI thread draws with.
struct Gpu<'a> {
    comp: &'a mut Compositor,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
}

/// The UI thread's side of a `gpu-live` stroke (the canvas's `GpuUi`).
struct Ui {
    layer: photocraft_doc::LayerId,
    readback: MaskReadback,
    backlog: VecDeque<Asked>,
}

impl Ui {
    /// Stamp the dabs and queue the requests, in order; the requests go to the read-back as far as it
    /// has room, all of them with `all`.
    fn run(&mut self, g: &mut Gpu<'_>, work: Vec<Work>, all: bool) -> Result<(), Box<dyn std::error::Error>> {
        for item in work {
            match item {
                Work::Dabs(dabs) if dabs.is_empty() => {}
                Work::Dabs(dabs) => {
                    let mut enc = g.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("bench_stamp") });
                    g.comp.stamp_dabs(g.device, g.queue, &mut enc, self.layer, &dabs)?;
                    g.queue.submit([enc.finish()]);
                }
                Work::Read(asked) => {
                    if !asked.0.is_empty() {
                        self.backlog.push_back(asked);
                    }
                }
            }
        }
        while let Some((keys, epoch)) = self.backlog.front_mut() {
            let taken = if all {
                self.readback.request_all(g.comp, g.device, g.queue, self.layer, keys, *epoch);
                keys.len()
            } else {
                self.readback.request(g.comp, g.device, g.queue, self.layer, keys, *epoch)
            };
            keys.drain(..taken);
            if keys.is_empty() {
                self.backlog.pop_front();
            } else {
                break;
            }
        }
        Ok(())
    }
}

/// `cpu-live` and `gpu-live`: the stroke frame by frame as the canvas draws it, then the pen-up.
fn live_run(
    gpu_dabs: bool,
    g: &mut Gpu<'_>,
    s: &mut Session,
    mut params: serde_json::Value,
    pts: &[[f64; 3]],
    pace: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut display = Display { tex: None };
    // Warm residency, untimed, as an open canvas has.
    {
        let doc = s.active().ok_or("no document")?.doc.clone();
        g.comp.render(g.device, g.queue, &doc, doc.bounds(), |e, o| display.sink(g.device, e, o))?;
        wait(g.device)?;
    }
    params["points"] = json!([pts[0]]);
    let mut ui = None;
    let (live, mut shown) = if gpu_dabs {
        let (live, first) = LiveStroke::begin_gpu(s, "paint.stroke", &params)?;
        let preview = live.gpu_preview().map_err(|why| format!("the stroke is not a GPU stroke: {why}"))?;
        let precision = g.comp.begin_external_mask(g.device, preview.doc.id, preview.layer, preview.doc.bounds(), None);
        println!("mask_precision={precision:?}");
        let mut state = Ui { layer: preview.layer, readback: MaskReadback::new(), backlog: VecDeque::new() };
        state.run(g, vec![Work::Dabs(first.iter().map(gpu_dab).collect())], false)?;
        ui = Some(state);
        (live, preview.doc)
    } else {
        let live = LiveStroke::begin_overlay(s, "paint.stroke", &params)?;
        if !live.is_overlay() {
            return Err("the stroke is not an overlay".into());
        }
        let doc = live.doc.clone();
        (live, doc)
    };
    let seed = live.seed;
    let read = ui.as_ref().map(|u| Read { readback: u.readback.clone(), device: g.device.clone() });
    let (tx, from_ui) = channel();
    let (to_ui, rx) = channel();
    let worker = std::thread::spawn(move || paint_thread(live, read, from_ui, to_ui));

    // The stroke: a point a frame, each frame waiting at most STROKE_COMMIT_WAIT for its step.
    let (mut sent, mut stepped) = (0usize, 0usize);
    let mut frames = Vec::new();
    let (mut t_work, mut t_render) = (0.0, 0.0);
    for p in &pts[1..] {
        let frame = Instant::now();
        tx.send(Down::Points(vec![StrokePoint::new(p[0], p[1], p[2] as f32)]))?;
        sent += 1;
        let mut damage = Rect::EMPTY;
        let mut waited = false;
        loop {
            let message = if !waited && stepped < sent {
                waited = true;
                rx.recv_timeout(STROKE_COMMIT_WAIT).ok()
            } else {
                rx.try_recv().ok()
            };
            let Some(Up::Step(step)) = message else { break };
            stepped += step.points;
            damage = damage.union(&step.damage);
            if let Some(doc) = step.doc {
                shown = doc;
            }
            if let Some(ui) = ui.as_mut() {
                let t = Instant::now();
                ui.run(g, step.work, false)?;
                t_work += ms(t);
            }
        }
        let t = Instant::now();
        if !damage.is_empty() {
            g.comp.render(g.device, g.queue, &shown, damage, |e, o| display.sink(g.device, e, o))?;
        }
        wait(g.device)?;
        t_render += ms(t);
        frames.push(ms(frame));
        if let Some(rest) = pace.checked_sub(frame.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    // Everything the thread has painted is shown before the pen goes up.
    while stepped < sent {
        let Ok(Up::Step(step)) = rx.recv() else { return Err("the paint thread stopped".into()) };
        stepped += step.points;
        if let Some(ui) = ui.as_mut() {
            ui.run(g, step.work, false)?;
        }
        if let Some(doc) = step.doc {
            shown = doc;
        }
        if !step.damage.is_empty() {
            g.comp.render(g.device, g.queue, &shown, step.damage, |e, o| display.sink(g.device, e, o))?;
        }
        wait(g.device)?;
    }

    // Pen-up: the release frame serves the thread's last work and waits for the commit as the canvas does.
    params["points"] = json!(pts);
    params["seed"] = json!(seed);
    let pen_up = Instant::now();
    tx.send(Down::Finish)?;
    let mut commit = None;
    let mut serve = |message: Up, g: &mut Gpu<'_>| -> Result<Option<(LiveCommit, Rect)>, Box<dyn std::error::Error>> {
        match message {
            Up::Final(work) => {
                let Some(ui) = ui.as_mut() else { return Err("a CPU stroke has no final work".into()) };
                ui.run(g, work, true)?;
                tx.send(Down::Served(true))?;
                Ok(None)
            }
            Up::Done(result) => Ok(Some(result?)),
            Up::Step(_) => Ok(None),
        }
    };
    let deadline = pen_up + STROKE_COMMIT_WAIT;
    while commit.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok(m) => commit = serve(m, g)?,
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => return Err("the paint thread stopped".into()),
        }
    }
    let release = ms(pen_up);
    // Frames that show the live document go by until the commit is ready.
    while commit.is_none() {
        match rx.recv_timeout(FRAME) {
            Ok(m) => commit = serve(m, g)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("the paint thread stopped".into()),
        }
    }
    let (commit, overlay) = commit.ok_or("no commit")?;
    let (landing, mb) = land(s, commit, params, overlay, (&mut *g.comp, g.device, g.queue, &mut display))?;
    if let Some(ui) = ui.as_ref() {
        g.comp.clear_external_mask(ui.layer);
    }
    let _ = worker.join();
    let committed = ms(pen_up);
    let n = frames.len();
    let total: f64 = frames.iter().sum();
    frames.sort_by(f64::total_cmp);
    let pct = |p: f64| frames[(((n - 1) as f64) * p).round() as usize];
    println!(
        "mode={} frames={n}\n\
         frame_ms mean={:.3} p95={:.3} max={:.3} (stamps+requests {t_work:.1} total, render+wait {t_render:.1} total)\n\
         release_frame_ms={release:.2}\n\
         landing_frame_ms={landing:.2} (uploads {mb:.1} MB)\n\
         pen_up_to_committed_ms={committed:.1}",
        if gpu_dabs { "gpu-live" } else { "cpu-live" },
        total / n as f64,
        pct(0.95),
        pct(1.0),
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map_or("async", String::as_str).to_string();
    let size: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100.0);
    let hardness: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let dabs: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4000);
    if !matches!(mode.as_str(), "sync" | "async" | "cpu-live" | "gpu-live") {
        return Err("mode is sync, async, cpu-live or gpu-live".into());
    }
    let pace_ms: f64 = args.iter().position(|a| a == "--pace-ms").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(8.0);

    let adapter = pollster::block_on(
        wgpu::Instance::default()
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
    )?;
    // The GPU coverage is R32F where the device blends it (the canvas asks for the same).
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wgpu::Features::FLOAT32_BLENDABLE,
        required_limits: adapter.limits(),
        ..Default::default()
    }))?;
    let info = adapter.get_info();
    println!("adapter={:?} backend={:?} type={:?}", info.name, info.backend, info.device_type);
    let mut comp = Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter))?;

    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "background": "white"}))?;
    s.execute("layer.new.layer", json!({}))?;
    s.edit("texture", |doc, active| {
        let Some(surf) = active.and_then(|id| doc.layer_mut(id)).and_then(|l| l.surface_mut()) else { return Ok(()) };
        let fmt = surf.format();
        for y0 in (0..H).step_by(128) {
            let r = Rect::new(0, y0, W, (y0 + 128).min(H));
            let mut data = Vec::new();
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    let v = ((x * 7 + y * 13) % 251) as f32 / 250.0;
                    let g = (x as f32 / W as f32 + (y % 97) as f32 / 400.0).fract();
                    data.extend(photocraft_raster::from_rgba(&fmt, [v, g, ((x ^ y) % 64) as f32 / 63.0, 1.0]));
                }
            }
            surf.write_region(r, &data);
        }
        Ok(())
    })?;

    let mut params = json!({"size": size, "hardness": hardness, "spacing": 0.25, "color": "#ff0000", "smoothing": 0.0, "target": "pixels"});
    let (w, h) = (f64::from(W), f64::from(H));
    let step = size * 0.25;
    let margin = size;
    let span = h - 2.0 * margin;
    let segments = ((dabs as f64 * step) / span).ceil().max(1.0) as usize;
    let dx = (w - 2.0 * margin) / segments as f64;
    let mut pts: Vec<[f64; 3]> = vec![[margin, margin, 1.0]];
    let mut pos = (margin, margin);
    'path: for seg in 0..segments {
        let (x0, y0) = pos;
        let (x1, y1) = (margin + (seg + 1) as f64 * dx, if seg % 2 == 0 { h - margin } else { margin });
        let n = (((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt() / step).ceil().max(1.0) as usize;
        for i in 1..=n {
            let f = i as f64 / n as f64;
            pts.push([x0 + (x1 - x0) * f, y0 + (y1 - y0) * f, 1.0]);
            if pts.len() > dabs {
                break 'path;
            }
        }
        pos = (x1, y1);
    }

    if mode.ends_with("-live") {
        let mut g = Gpu { comp: &mut comp, device: &device, queue: &queue };
        return live_run(mode == "gpu-live", &mut g, &mut s, params, &pts, Duration::from_secs_f64(pace_ms / 1e3));
    }
    // The stroke as the canvas showed it at pen-up: painted, and its live document on the GPU.
    params["points"] = json!([pts[0]]);
    let mut live = LiveStroke::begin_overlay(&s, "paint.stroke", &params)?;
    if !live.is_overlay() {
        return Err("the stroke is not an overlay".into());
    }
    let rest: Vec<StrokePoint> = pts[1..].iter().map(|p| StrokePoint::new(p[0], p[1], p[2] as f32)).collect();
    live.push(&rest)?;
    let mut display = Display { tex: None };
    comp.render(&device, &queue, &live.doc, live.doc.bounds(), |e, o| display.sink(&device, e, o))?;
    wait(&device)?;
    params["points"] = json!(pts);
    params["seed"] = json!(live.seed);
    let overlay = live.overlay_bounds().unwrap_or(Rect::EMPTY);

    let pen_up = Instant::now();
    let (release, landing, mb) = if mode == "sync" {
        let commit = live.into_commit()?;
        let (t, mb) = land(&mut s, commit, params, overlay, (&mut comp, &device, &queue, &mut display))?;
        (ms(pen_up), t, mb)
    } else {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(live.into_commit());
        });
        let mut ready = rx.recv_timeout(STROKE_COMMIT_WAIT).ok();
        let release = ms(pen_up);
        // Frames that show the live document: nothing changed on the GPU, so they draw nothing new.
        while ready.is_none() {
            ready = rx.recv_timeout(FRAME).ok();
        }
        let commit = ready.ok_or("no commit")??;
        let (t, mb) = land(&mut s, commit, params, overlay, (&mut comp, &device, &queue, &mut display))?;
        (release, t, mb)
    };
    let committed = ms(pen_up);
    println!(
        "mode={mode} size={size} hardness={hardness} dabs={dabs}\n\
         release_frame_ms={release:.2}\n\
         landing_frame_ms={landing:.2} (uploads {mb:.1} MB)\n\
         pen_up_to_committed_ms={committed:.1}"
    );
    Ok(())
}
