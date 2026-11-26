//! A live Brush stroke end to end on the real GPU, the blended-pixels path (`old`,
//! `LiveStroke::begin_with`) against the masked colour layer overlay (`new`,
//! `LiveStroke::begin_overlay`): per frame the stroke's `push`, the compositor's `render` of the
//! live document over the step's damage, and the wait for the GPU; then the pen-up commit.
//!
//! `cargo run --release -p photocraft-automation --features gpu --example bench_live_stroke_gpu -- <old|new|gpu> <size> <hardness> <dabs> [--per-frame N] [--sanity] [--half] [--pace-ms MS]`
//!
//! `coverage <size> <hardness> <points> [--flow F] [--roundness R] [--angle DEG]` instead stamps a
//! stroke on the GPU in R32F (where the device blends it) and in R16F and prints how far each is
//! from the CPU's coverage of the same dabs.
//!
//! The document is 4928x3280 with a white background and a pixel layer of non-uniform content
//! over the whole canvas. The stroke is `stroke_bench`'s zigzag at spacing 0.25, one point per
//! frame, so about one frame per dab. The render sink copies each finished chunk into a
//! 2048x1536 display-sized texture (same format as the chunk, extent clamped), as a canvas
//! blit would. `--sanity` reads the live document back from the GPU before the pen-up and
//! compares it with the committed document's GPU render. `--per-frame N` feeds N points per
//! frame. The pen-up renders what the canvas redraws at release: the relayout damage, plus an
//! overlay stroke's bounds; `stroke_incl_pen_up_ms` is the frame total plus the pen-up.
use photocraft_engine::Session;
use photocraft_engine::brush_cmds::{LiveStroke, LiveStrokeExt};
use photocraft_engine::paint::dynamics::DabGenerator;
use photocraft_engine::paint::{BrushContext, BrushSettings, StrokePoint, StrokeRenderer};
use photocraft_geom::Rect;
use photocraft_gpu::{Compositor, GpuDab, GpuTimer, MaskPrecision};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Instant;

const W: i32 = 4928;
const H: i32 = 3280;

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

fn pct(sorted: &[f64], p: f64) -> f64 {
    sorted[(((sorted.len() - 1) as f64) * p).round() as usize]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map_or("new", String::as_str).to_string();
    let size: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100.0);
    let hardness: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let dabs: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4000);
    let sanity = args.iter().any(|a| a == "--sanity");
    // Points fed per frame (the canvas hands the paint thread what arrived since the last frame).
    let per_frame: usize = args.iter().position(|a| a == "--per-frame").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let half = args.iter().any(|a| a == "--half");
    assert!(matches!(mode.as_str(), "old" | "new" | "gpu" | "coverage"), "mode is old, new, gpu or coverage");
    let option = |name: &str, default: f64| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(default);
    let pace = std::time::Duration::from_secs_f64(option("--pace-ms", 0.0) / 1e3);

    let adapter = pollster::block_on(
        wgpu::Instance::default()
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
    )?;
    let timing = adapter.features().contains(photocraft_gpu::timing::FEATURES);
    let blendable = adapter.features() & wgpu::Features::FLOAT32_BLENDABLE;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: (if timing { photocraft_gpu::timing::FEATURES } else { wgpu::Features::empty() }) | blendable,
        required_limits: adapter.limits(),
        ..Default::default()
    }))?;
    let info = adapter.get_info();
    println!("adapter={:?} backend={:?} type={:?} timestamps={timing}", info.name, info.backend, info.device_type);
    let mut comp = Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter))?;
    let measured = comp.measure_gpu_time(&device, &queue);
    if mode == "coverage" {
        let brush = BrushSettings {
            size: size as f32,
            hardness: hardness as f32,
            flow: option("--flow", 1.0) as f32,
            roundness: option("--roundness", 1.0) as f32,
            angle: option("--angle", 0.0) as f32,
            spacing: 0.1,
            ..BrushSettings::default()
        };
        return coverage(&Gpu { device: &device, queue: &queue }, &mut comp, &brush, dabs);
    }

    // The document: white background, a pixel layer of photo-like content over every tile.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "background": "white"}))?;
    s.execute("layer.new.layer", json!({}))?;
    s.edit("texture", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
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
    // The zigzag of stroke_bench: top margin to bottom and back, advancing across the width.
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

    let mut display = Display { tex: None };
    // Warm residency: one full render, untimed, as an open canvas already has.
    {
        let doc = s.active().unwrap().doc.clone();
        comp.render(&device, &queue, &doc, doc.bounds(), |e, o| display.sink(&device, e, o))?;
        wait(&device)?;
        let _ = comp.take_gpu_timing();
    }

    params["points"] = json!([pts[0]]);
    if mode == "gpu" {
        let run = GpuRun { size, hardness, dabs, per_frame, sanity, precision: half.then_some(MaskPrecision::Half), measured, pace };
        let mut frames = Frames { gpu: Gpu { device: &device, queue: &queue }, comp: &mut comp, display: &mut display };
        return run_gpu(&run, &mut frames, &mut s, params, &pts);
    }
    let mut live = if mode == "old" { LiveStroke::begin_with(&s, "paint.stroke", &params)? } else { LiveStroke::begin_overlay(&s, "paint.stroke", &params)? };
    assert_eq!(live.is_overlay(), mode == "new", "the stroke is not shown as the requested mode");

    let (mut t_push, mut t_render, mut t_wait) = (0.0, 0.0, 0.0);
    let mut frames = Vec::new();
    let (mut tiles, mut bytes, mut passes, mut chunks) = (0usize, 0usize, 0usize, 0usize);
    let (mut gpu_ms, mut gpu_n) = (0.0, 0usize);
    for chunk in pts[1..].chunks(per_frame) {
        let frame = Instant::now();
        let t = Instant::now();
        let step: Vec<StrokePoint> = chunk.iter().map(|p| StrokePoint::new(p[0], p[1], p[2] as f32)).collect();
        let damage = live.push(&step)?;
        let a = ms(t);
        let doc = live.doc.clone();
        let t = Instant::now();
        let stats = comp.render(&device, &queue, &doc, damage, |e, o| display.sink(&device, e, o))?;
        let b = ms(t);
        let t = Instant::now();
        wait(&device)?;
        let c = ms(t);
        frames.push(ms(frame));
        t_push += a;
        t_render += b;
        t_wait += c;
        tiles += stats.tiles_uploaded;
        bytes += stats.bytes_uploaded;
        passes += stats.passes;
        chunks += stats.chunks;
        if let Some(g) = comp.take_gpu_timing() {
            gpu_ms += g.ms;
            gpu_n += 1;
        }
    }
    let live_pixels = if sanity { Some(photocraft_gpu::render_to_vec(&mut comp, &device, &queue, &live.doc, live.doc.bounds())?) } else { None };

    // Pen-up: the commit takes the live stroke's pixels, then the canvas shows the committed doc.
    params["points"] = json!(pts);
    params["seed"] = json!(live.seed);
    // What the canvas redraws at release (`commit_brush_stroke`): where laying effects out again moved
    // a frame, and an overlay stroke's bounds, which showed no dither.
    let overlay_bounds = live.overlay_bounds();
    let before = s.active().unwrap().doc.clone();
    let t = Instant::now();
    s.offer_live_stroke(live)?;
    let result = s.execute("paint.stroke", params)?;
    let commit = ms(t);
    let doc = s.active().unwrap().doc.clone();
    let damage_up = photocraft_compose::relayout_damage(&before, &doc).union(&overlay_bounds.unwrap_or(Rect::EMPTY));
    let t = Instant::now();
    let (up_tiles, up_bytes) = if damage_up.is_empty() {
        (0, 0)
    } else {
        let stats = comp.render(&device, &queue, &doc, damage_up, |e, o| display.sink(&device, e, o))?;
        wait(&device)?;
        (stats.tiles_uploaded, stats.bytes_uploaded)
    };
    let render_up = ms(t);
    let pen_up = commit + render_up;
    assert!(result.get("fromLive").is_some(), "the commit did not take the live pixels");

    let n = frames.len();
    let total: f64 = frames.iter().sum();
    let mean = total / n as f64;
    let mut sorted = frames.clone();
    sorted.sort_by(f64::total_cmp);
    println!(
        "mode={mode} size={size} hardness={hardness} dabs={dabs} per_frame={per_frame} frames={n}\n\
         totals_ms push={t_push:.1} render={t_render:.1} gpu_wait={t_wait:.1} frame_total={total:.1}\n\
         frame_ms mean={mean:.3} p95={:.3} max={:.3}\n\
         pen_up_ms total={pen_up:.1} (offer+commit={commit:.1}, render+wait={render_up:.1}; render uploads {} tiles {:.1} MB)\n\
         stroke_incl_pen_up_ms={:.1}\n\
         gpu_side_ms={gpu_ms:.1} over {gpu_n} of {n} frames (timing on={measured})\n\
         upload tiles={tiles} bytes={bytes} MB={:.1} passes={passes} chunks={chunks}",
        pct(&sorted, 0.95),
        sorted[n - 1],
        up_tiles,
        up_bytes as f64 / 1e6,
        total + pen_up,
        bytes as f64 / 1e6,
    );
    if let Some(live_pixels) = live_pixels {
        let committed = photocraft_gpu::render_to_vec(&mut comp, &device, &queue, &doc, doc.bounds())?;
        let worst = live_pixels
            .iter()
            .zip(&committed)
            .map(|(p, q)| (0..3).map(|c| (p[c] * p[3] - q[c] * q[3]).abs()).chain([(p[3] - q[3]).abs()]).fold(0.0f32, f32::max))
            .fold(0.0f32, f32::max);
        println!("sanity worst premultiplied difference live vs committed = {:.3}/255", worst * 255.0);
    }
    Ok(())
}

/// The `gpu` mode's settings.
struct GpuRun {
    size: f64,
    hardness: f64,
    dabs: usize,
    per_frame: usize,
    sanity: bool,
    precision: Option<MaskPrecision>,
    measured: bool,
    /// The time a frame takes at the pointer's pace (0: frames run back to back).
    pace: std::time::Duration,
}

/// A device and its queue.
struct Gpu<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
}

/// What a frame draws with.
struct Frames<'a> {
    gpu: Gpu<'a>,
    comp: &'a mut Compositor,
    display: &'a mut Display,
}

/// The points handed to the paint thread, and how many dabs they make.
struct Handover {
    pts: Vec<StrokePoint>,
    dabs: usize,
}

/// A dab as the stamping shader takes it.
fn gpu_dab(g: &photocraft_engine::paint::TipGeometry) -> GpuDab {
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

/// The dabs `pts` make (the stroke's own generator, as `StrokeRenderer::push` feeds it), as the
/// stamping shader takes them, and how many dabs that is.
fn next_dabs(generator: &mut DabGenerator, context: &BrushContext, pts: &[StrokePoint]) -> (Vec<GpuDab>, usize) {
    let (mut dabs, mut dual) = (Vec::new(), Vec::new());
    generator.push(pts, &mut dabs, &mut dual);
    (dabs.iter().filter_map(|d| context.tip_geometry(d)).map(|g| gpu_dab(&g)).collect(), dabs.len())
}

/// The CPU coverage the commit needs, built from the points as they are handed over. Returns the stroke and when it finished its last hand-over.
fn paint_thread(mut live: LiveStroke, rx: mpsc::Receiver<Handover>, done_dabs: std::sync::Arc<AtomicUsize>) -> (LiveStroke, Instant) {
    let mut last_done = Instant::now();
    let push = |live: &mut LiveStroke, m: Handover| {
        let _ = live.push(&m.pts);
        done_dabs.fetch_add(m.dabs, Ordering::Release);
        Instant::now()
    };
    while let Ok(m) = rx.recv() {
        last_done = push(&mut live, m);
    }
    (live, last_done)
}

fn run_gpu(run: &GpuRun, g: &mut Frames<'_>, s: &mut Session, mut params: serde_json::Value, pts: &[[f64; 3]]) -> Result<(), Box<dyn std::error::Error>> {
    let (device, queue) = (g.gpu.device, g.gpu.queue);
    let live = LiveStroke::begin_overlay(s, "paint.stroke", &params)?;
    let preview = live.gpu_preview().map_err(|why| format!("the gpu mode cannot show this stroke: {why}"))?;
    let canvas = preview.doc.bounds();
    let precision = g.comp.begin_external_mask(device, preview.doc.id, preview.layer, canvas, run.precision);
    println!("mask_precision={precision:?}");
    let context = BrushContext::new(&preview.brush);
    let mut generator = DabGenerator::new(&preview.brush, 1.0);
    let mut stamp_timer = GpuTimer::new(device, queue);
    let seed = live.seed;

    let (tx, rx) = mpsc::channel::<Handover>();
    let done_dabs = std::sync::Arc::new(AtomicUsize::new(0));
    let painter = {
        let done = done_dabs.clone();
        std::thread::spawn(move || paint_thread(live, rx, done))
    };

    let (mut t_gen, mut t_stamp, mut t_render, mut t_wait) = (0.0, 0.0, 0.0, 0.0);
    let mut frames = Vec::new();
    let (mut stamp_gpu_ms, mut stamp_gpu_n) = (0.0, 0usize);
    let (mut gpu_ms, mut gpu_n) = (0.0, 0usize);
    let (mut tiles, mut passes, mut quads) = (0usize, 0usize, 0usize);
    let (mut sent_dabs, mut stamped_dabs) = (0usize, 0usize);
    let mut bounds = Rect::EMPTY;
    let mut behind = 0usize;
    let mut last_sent = Instant::now();
    // The stroke's first point went to the CPU when it began: its dabs are the CPU's already.
    let first = [StrokePoint::new(pts[0][0], pts[0][1], pts[0][2] as f32)];
    let (mut pending, _) = next_dabs(&mut generator, &context, &first);
    for chunk in pts[1..].chunks(run.per_frame) {
        let frame = Instant::now();
        let t = Instant::now();
        let step: Vec<StrokePoint> = chunk.iter().map(|p| StrokePoint::new(p[0], p[1], p[2] as f32)).collect();
        let (made_dabs, made) = next_dabs(&mut generator, &context, &step);
        sent_dabs += made;
        tx.send(Handover { pts: step, dabs: made })?;
        last_sent = Instant::now();
        pending.extend(made_dabs);
        let gpu_dabs = std::mem::take(&mut pending);
        let damage = gpu_dabs.iter().fold(Rect::EMPTY, |r, d| r.union(&d.rect)).intersect(&canvas);
        let a = ms(t);
        let t = Instant::now();
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("bench_stamp") });
        if let Some(timer) = stamp_timer.as_mut() {
            timer.start(&mut enc, u64::from(damage.width()) * u64::from(damage.height()));
        }
        let st = g.comp.stamp_dabs(device, queue, &mut enc, preview.layer, &gpu_dabs)?;
        if let Some(timer) = stamp_timer.as_mut() {
            timer.finish(&mut enc);
        }
        queue.submit([enc.finish()]);
        if let Some(timer) = stamp_timer.as_mut() {
            timer.submitted();
        }
        let b = ms(t);
        stamped_dabs += gpu_dabs.len();
        quads += st.quads;
        let t = Instant::now();
        let stats = g.comp.render(device, queue, &preview.doc, damage, |e, o| g.display.sink(device, e, o))?;
        let c = ms(t);
        let t = Instant::now();
        wait(device)?;
        let d = ms(t);
        frames.push(ms(frame));
        if let Some(rest) = run.pace.checked_sub(frame.elapsed()) {
            std::thread::sleep(rest);
        }
        if let Some(timer) = stamp_timer.as_mut()
            && let Some(m) = timer.take()
        {
            stamp_gpu_ms += m.ms;
            stamp_gpu_n += 1;
        }
        if let Some(m) = g.comp.take_gpu_timing() {
            gpu_ms += m.ms;
            gpu_n += 1;
        }
        t_gen += a;
        t_stamp += b;
        t_render += c;
        t_wait += d;
        tiles += stats.tiles_uploaded;
        passes += stats.passes;
        bounds = bounds.union(&damage);
        behind = sent_dabs.saturating_sub(done_dabs.load(Ordering::Acquire));
    }
    let live_pixels = if run.sanity { Some(photocraft_gpu::render_to_vec(g.comp, device, queue, &preview.doc, canvas)?) } else { None };

    // Pen-up: wait for the CPU coverage, commit, then the canvas shows the committed doc.
    let t = Instant::now();
    drop(tx);
    let (live, last_done) = painter.join().map_err(|_| "the paint thread panicked")?;
    let cpu_wait = ms(t);
    let lag_ms = last_done.saturating_duration_since(last_sent).as_secs_f64() * 1e3;
    params["points"] = json!(pts);
    params["seed"] = json!(seed);
    let t = Instant::now();
    s.offer_live_stroke(live)?;
    let result = s.execute("paint.stroke", params)?;
    let commit = ms(t);
    let doc = s.active().ok_or("no document")?.doc.clone();
    let t = Instant::now();
    let stats = g.comp.render(device, queue, &doc, bounds, |e, o| g.display.sink(device, e, o))?;
    wait(device)?;
    let render_up = ms(t);
    let pen_up = cpu_wait + commit + render_up;
    assert!(result.get("fromLive").is_some(), "the commit did not take the live pixels");

    let n = frames.len();
    let total: f64 = frames.iter().sum();
    let mean = total / n as f64;
    let mut sorted = frames.clone();
    sorted.sort_by(f64::total_cmp);
    println!(
        "mode=gpu size={} hardness={} dabs={} per_frame={} frames={n} stamped_dabs={stamped_dabs} cpu_dabs={sent_dabs} quads={quads}\n\
         totals_ms gen={t_gen:.1} stamp_encode_submit={t_stamp:.1} render={t_render:.1} gpu_wait={t_wait:.1} frame_total={total:.1}\n\
         frame_ms mean={mean:.3} p95={:.3} max={:.3}\n\
         stamp_gpu_ms={stamp_gpu_ms:.2} over {stamp_gpu_n} of {n} frames (timing on={})\n\
         cpu_lag_at_last_frame dabs={behind} ms={lag_ms:.1}\n\
         pen_up_ms total={pen_up:.1} (cpu_wait={cpu_wait:.1}, offer+commit={commit:.1}, render+wait={render_up:.1}; render uploads {} tiles {:.1} MB)\n\
         stroke_incl_pen_up_ms={:.1}\n\
         gpu_side_ms={gpu_ms:.1} over {gpu_n} of {n} frames (timing on={})\n\
         upload tiles={tiles} passes={passes}",
        run.size,
        run.hardness,
        run.dabs,
        run.per_frame,
        pct(&sorted, 0.95),
        sorted[n - 1],
        stamp_timer.is_some(),
        stats.tiles_uploaded,
        stats.bytes_uploaded as f64 / 1e6,
        total + pen_up,
        run.measured,
    );
    g.comp.clear_external_mask(preview.layer);
    if let Some(live_pixels) = live_pixels {
        let committed = photocraft_gpu::render_to_vec(g.comp, device, queue, &doc, doc.bounds())?;
        let worst = live_pixels
            .iter()
            .zip(&committed)
            .map(|(p, q)| (0..3).map(|c| (p[c] * p[3] - q[c] * q[3]).abs()).chain([(p[3] - q[3]).abs()]).fold(0.0f32, f32::max))
            .fold(0.0f32, f32::max);
        println!("sanity worst premultiplied difference live vs committed = {:.3}/255", worst * 255.0);
    }
    Ok(())
}

/// The GPU's coverage of a stroke's dabs in each mask precision against the CPU's.
fn coverage(g: &Gpu<'_>, comp: &mut Compositor, brush: &BrushSettings, points: usize) -> Result<(), Box<dyn std::error::Error>> {
    let (device, queue) = (g.device, g.queue);
    let (w, h) = (2000, 1400);
    let pts: Vec<StrokePoint> =
        (0..points).map(|i| StrokePoint::new(150.0 + i as f64 * 1700.0 / points as f64, 700.0 + (i as f64 * 0.21).sin() * 450.0, 1.0)).collect();
    let mut renderer = StrokeRenderer::new(brush, None, 1.0).record_dabs();
    renderer.push(&pts);
    renderer.finish();
    let canvas = Rect::new(0, 0, w, h);
    let cpu = renderer.coverage_in(canvas);
    let context = BrushContext::new(brush);
    let gpu_dabs: Vec<GpuDab> = renderer.dabs().iter().filter_map(|d| context.tip_geometry(d)).map(|t| gpu_dab(&t)).collect();
    println!("dabs={} size={} hardness={} flow={} roundness={} angle={}", gpu_dabs.len(), brush.size, brush.hardness, brush.flow, brush.roundness, brush.angle);
    let mut precisions = vec![MaskPrecision::Half];
    if Compositor::full_precision_masks(device) {
        precisions.insert(0, MaskPrecision::Full);
    }
    for precision in precisions {
        let layer = photocraft_doc::LayerId::fresh();
        comp.begin_external_mask(device, photocraft_doc::DocId(1), layer, canvas, Some(precision));
        for chunk in gpu_dabs.chunks(8) {
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("coverage") });
            comp.stamp_dabs(device, queue, &mut enc, layer, chunk)?;
            queue.submit([enc.finish()]);
        }
        let mut mask = vec![0.0f32; (w * h) as usize];
        for (rect, texture) in comp.external_mask_cells(layer) {
            let half = texture.format() == wgpu::TextureFormat::R16Float;
            let (cw, ch) = (rect.width(), rect.height());
            let row = (cw * if half { 2 } else { 4 }).next_multiple_of(256);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("coverage readback"),
                size: u64::from(row * ch),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("coverage readback") });
            enc.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(ch) },
                },
                wgpu::Extent3d { width: cw, height: ch, depth_or_array_layers: 1 },
            );
            queue.submit([enc.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            wait(device)?;
            let data = buffer.slice(..).get_mapped_range()?;
            for y in 0..ch as usize {
                for x in 0..cw as usize {
                    let v = if half {
                        let at = y * row as usize + x * 2;
                        photocraft_gpu::f16_to_f32(u16::from_le_bytes([data[at], data[at + 1]]))
                    } else {
                        let at = y * row as usize + x * 4;
                        f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
                    };
                    let (px, py) = (rect.x0 + x as i32, rect.y0 + y as i32);
                    if px < w && py < h {
                        mask[(py * w + px) as usize] = v;
                    }
                }
            }
        }
        let mut diffs: Vec<f32> = mask.iter().zip(&cpu).filter(|(a, b)| **a > 0.0 || **b > 0.0).map(|(a, b)| (a - b).abs()).collect();
        diffs.sort_by(f32::total_cmp);
        let n = diffs.len().max(1);
        let mean = diffs.iter().sum::<f32>() / n as f32;
        println!(
            "{precision:?}: painted_pixels={} worst={:.2e} p99.9={:.2e} mean={mean:.2e}",
            diffs.len(),
            diffs.last().copied().unwrap_or(0.0),
            diffs.get(n * 999 / 1000).copied().unwrap_or(0.0),
        );
        comp.clear_external_mask(layer);
    }
    Ok(())
}
