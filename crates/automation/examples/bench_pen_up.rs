//! How long releasing the pen holds up the UI thread for an overlay Brush stroke
//! (`LiveStroke::begin_overlay`), with the commit's composite on the UI thread (`sync`, as #17082
//! left it) or on the stroke's paint thread (`async`, the canvas's `PendingStroke`).
//!
//! `cargo run --release -p photocraft-automation --features gpu --example bench_pen_up -- <sync|async> <size> <hardness> <dabs>`
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
//! Printed: `release_frame_ms` (the UI thread's time from pen-up to the next frame it can draw),
//! `landing_frame_ms` (async: the frame that shows the committed pixels) and
//! `pen_up_to_committed_ms` (from pen-up until the committed pixels are on the GPU).
use photocraft_engine::Session;
use photocraft_engine::brush_cmds::{LiveStroke, LiveStrokeExt};
use photocraft_engine::paint::StrokePoint;
use photocraft_geom::Rect;
use photocraft_gpu::Compositor;
use serde_json::json;
use std::time::{Duration, Instant};

const W: i32 = 4928;
const H: i32 = 3280;
/// The canvas's wait for the commit in the release frame (`PhotocraftApp::stroke_commit_wait`).
const STROKE_COMMIT_WAIT: Duration = Duration::from_millis(6);
/// How often the UI thread draws a frame while the commit is pending (a 60 Hz display).
const FRAME: Duration = Duration::from_micros(16_667);

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map_or("async", String::as_str).to_string();
    let size: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100.0);
    let hardness: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let dabs: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4000);
    if !matches!(mode.as_str(), "sync" | "async") {
        return Err("mode is sync or async".into());
    }

    let adapter = pollster::block_on(
        wgpu::Instance::default()
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() }))?;
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
