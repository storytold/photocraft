//! A live Brush stroke end to end on the real GPU, the blended-pixels path (`old`,
//! `LiveStroke::begin_with`) against the masked colour layer overlay (`new`,
//! `LiveStroke::begin_overlay`): per frame the stroke's `push`, the compositor's `render` of the
//! live document over the step's damage, and the wait for the GPU; then the pen-up commit.
//!
//! `cargo run --release -p photocraft-automation --features gpu --example bench_live_stroke_gpu -- <old|new> <size> <hardness> <dabs> [--per-frame N] [--settle] [--sanity]`
//!
//! The document is 4928x3280 with a white background and a pixel layer of non-uniform content
//! over the whole canvas. The stroke is `stroke_bench`'s zigzag at spacing 0.25, one point per
//! frame, so about one frame per dab. The render sink copies each finished chunk into a
//! 2048x1536 display-sized texture (same format as the chunk, extent clamped), as a canvas
//! blit would. `--sanity` reads the live document back from the GPU before the pen-up and
//! compares it with the committed document's GPU render. `--per-frame N` feeds N points per
//! frame. `--settle` calls `LiveStroke::settle` between frames, 16 tiles at a time up to 64 tiles
//! a frame, as the paint thread does while the UI renders: its time is `settle_ms`, outside the
//! frame total, and the pen-up then has little left to composite.
use photocraft_engine::Session;
use photocraft_engine::brush_cmds::{LiveStroke, LiveStrokeExt};
use photocraft_engine::paint::StrokePoint;
use photocraft_geom::Rect;
use photocraft_gpu::Compositor;
use serde_json::json;
use std::time::Instant;

const W: i32 = 4928;
const H: i32 = 3280;
/// Tiles per `settle` call, and the most a frame's gap settles.
const SETTLE_BATCH: usize = 16;
const SETTLE_TILES_PER_FRAME: usize = 64;

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
    let settle = args.iter().any(|a| a == "--settle");
    assert!(matches!(mode.as_str(), "old" | "new"), "mode is old or new");

    let adapter = pollster::block_on(
        wgpu::Instance::default()
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
    )?;
    let timing = adapter.features().contains(photocraft_gpu::timing::FEATURES);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: if timing { photocraft_gpu::timing::FEATURES } else { wgpu::Features::empty() },
        required_limits: adapter.limits(),
        ..Default::default()
    }))?;
    let info = adapter.get_info();
    println!("adapter={:?} backend={:?} type={:?} timestamps={timing}", info.name, info.backend, info.device_type);
    let mut comp = Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter))?;
    let measured = comp.measure_gpu_time(&device, &queue);

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
    let mut live = if mode == "old" { LiveStroke::begin_with(&s, "paint.stroke", &params)? } else { LiveStroke::begin_overlay(&s, "paint.stroke", &params)? };
    assert_eq!(live.is_overlay(), mode == "new", "the stroke is not shown as the requested mode");

    let (mut t_push, mut t_render, mut t_wait, mut t_settle) = (0.0, 0.0, 0.0, 0.0);
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
        if settle {
            // Between frames, as the paint thread settles while the UI renders.
            let t = Instant::now();
            for _ in 0..SETTLE_TILES_PER_FRAME / SETTLE_BATCH {
                if !live.settle(SETTLE_BATCH) {
                    break;
                }
            }
            t_settle += ms(t);
        }
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
        "mode={mode} size={size} hardness={hardness} dabs={dabs} per_frame={per_frame} settle={settle} frames={n}\n\
         totals_ms push={t_push:.1} render={t_render:.1} gpu_wait={t_wait:.1} frame_total={total:.1}\n\
         settle_ms={t_settle:.1}\n\
         frame_ms mean={mean:.3} p95={:.3} max={:.3}\n\
         pen_up_ms total={pen_up:.1} (offer+commit={commit:.1}, render+wait={render_up:.1}; render uploads {} tiles {:.1} MB)\n\
         stroke_incl_pen_up_ms={:.1}\n\
         gpu_side_ms={gpu_ms:.1} over {gpu_n} of {n} frames (timing on={measured})\n\
         upload tiles={tiles} bytes={bytes} MB={:.1} passes={passes} chunks={chunks}",
        pct(&sorted, 0.95),
        sorted[n - 1],
        up_tiles,
        up_bytes as f64 / 1e6,
        total + t_settle + pen_up,
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
