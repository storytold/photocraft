//! Camera Raw's preview develop on the 858×572 proxy of a 6000×4000 image: the CPU develop
//! against the GPU passes (upload, render, readback), for per-pixel settings and for the spatial
//! stages (Dehaze, Shadows and Highlights, Clarity, Texture, Detail) with them. "Source uploaded every
//! time" also makes Dehaze's copy on the CPU every time, as a white balance drag does.
//! `cargo run --release -p photocraft-gpu --example bench_camera_raw [-- --quick]`
use std::time::Instant;

use photocraft_algo::camera_raw::{CameraRaw, DevelopInputs, Wheel, develop, per_pixel, spatial, spatial_tone, white_black_points};
use photocraft_gpu::CameraRawDevelop;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn time(runs: usize, mut f: impl FnMut()) -> f64 {
    f();
    median(
        (0..runs)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect(),
    )
}

/// [`time`] for a call that can fail: its first error ends the benchmark instead of being timed.
fn try_time<E>(runs: usize, mut f: impl FnMut() -> Result<(), E>) -> Result<f64, E> {
    let mut failed = None;
    let ms = time(runs, || {
        if failed.is_none() {
            failed = f().err();
        }
    });
    failed.map_or(Ok(ms), Err)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runs = if std::env::args().any(|a| a == "--quick") { 5 } else { 40 };
    let (w, h) = (858usize, 572usize);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let options = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
    let adapter = pollster::block_on(instance.request_adapter(&options))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let info = adapter.get_info();
    println!("adapter: {} ({:?}), proxy {w}x{h}, median of {runs}", info.name, info.backend);
    let built = Instant::now();
    let mut pass = CameraRawDevelop::new(&device)?;
    println!("pipeline build: {:.1} ms", built.elapsed().as_secs_f64() * 1e3);
    // A photo-like proxy: gradients, noise and bright patches, so the filter has edges to keep.
    let src: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
            let n = ((i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f32 / (1u64 << 24) as f32;
            if (i / w) % 90 < 25 && (i % w) % 130 < 40 {
                return [0.95, 0.92, 0.88, 1.0];
            }
            let c = [0.02 + 0.8 * x * x, 0.05 + 0.6 * y, 0.6 - 0.5 * x * y].map(|v| (v + (n - 0.5) * 0.05).clamp(0.0, 1.0));
            [c[0], c[1], c[2], 1.0]
        })
        .collect();
    let scale = w as f32 / 6000.0;
    let mut hsl = [0.0; 8];
    hsl[1] = 30.0;
    hsl[5] = -40.0;
    // `CameraRaw` is `#[non_exhaustive]`: each case is the base settings with some fields assigned, plus the job inputs.
    let inputs = DevelopInputs { pixel_scale: scale, ..Default::default() };
    let with = |base: &CameraRaw, set: &dyn Fn(&mut CameraRaw)| {
        let mut p = base.clone();
        set(&mut p);
        (p, inputs)
    };
    let none = CameraRaw::default();
    let mut per_pixel_stages = CameraRaw::default();
    per_pixel_stages.temperature = 20.0;
    per_pixel_stages.tint = 5.0;
    per_pixel_stages.exposure = 0.4;
    per_pixel_stages.contrast = 25.0;
    per_pixel_stages.whites = 30.0;
    per_pixel_stages.blacks = -20.0;
    per_pixel_stages.vibrance = 30.0;
    per_pixel_stages.saturation = 10.0;
    per_pixel_stages.curve_lights = 20.0;
    per_pixel_stages.point_curve = vec![[0.0, 8.0], [128.0, 140.0], [255.0, 250.0]];
    per_pixel_stages.hsl_hue = hsl;
    per_pixel_stages.hsl_sat = hsl;
    per_pixel_stages.hsl_lum = hsl;
    per_pixel_stages.grade_shadows = Wheel { hue: 210.0, sat: 30.0, lum: 0.0 };
    per_pixel_stages.grade_highlights = Wheel { hue: 40.0, sat: 20.0, lum: 5.0 };
    per_pixel_stages.vignette_amount = -30.0;
    per_pixel_stages.grain_amount = 25.0;
    let sets = [
        (
            "Basic (WB, Exposure, Contrast, Whites, Blacks)",
            with(&none, &|p| {
                p.temperature = 20.0;
                p.exposure = 0.4;
                p.contrast = 25.0;
                p.whites = 30.0;
                p.blacks = -20.0;
            }),
        ),
        ("every per-pixel stage", (per_pixel_stages.clone(), inputs)),
        ("Shadows +50", with(&none, &|p| p.shadows = 50.0)),
        ("Highlights -50", with(&none, &|p| p.highlights = -50.0)),
        (
            "Shadows +50, Highlights -50",
            with(&none, &|p| {
                p.shadows = 50.0;
                p.highlights = -50.0;
            }),
        ),
        (
            "Shadows +50, Highlights -50, every per-pixel stage",
            with(&per_pixel_stages, &|p| {
                p.shadows = 50.0;
                p.highlights = -50.0;
            }),
        ),
        ("Clarity +50", with(&none, &|p| p.clarity = 50.0)),
        ("Texture +50", with(&none, &|p| p.texture = 50.0)),
        ("Dehaze +40", with(&none, &|p| p.dehaze = 40.0)),
        (
            "Clarity +50, Texture +50, Dehaze +40, every per-pixel stage",
            with(&per_pixel_stages, &|p| {
                p.clarity = 50.0;
                p.texture = 50.0;
                p.dehaze = 40.0;
            }),
        ),
        (
            "Clarity, Texture, Dehaze, Shadows, Highlights, every per-pixel stage",
            with(&per_pixel_stages, &|p| {
                p.clarity = 50.0;
                p.texture = 50.0;
                p.dehaze = 40.0;
                p.shadows = 50.0;
                p.highlights = -50.0;
            }),
        ),
        ("Sharpening 40", with(&none, &|p| p.sharpen_amount = 40.0)),
        (
            "Sharpening 40, Masking 30",
            with(&none, &|p| {
                p.sharpen_amount = 40.0;
                p.sharpen_masking = 30.0;
            }),
        ),
        ("Noise Reduction 30", with(&none, &|p| p.noise_luminance = 30.0)),
        ("Color Noise Reduction 25", with(&none, &|p| p.noise_color = 25.0)),
        (
            "Sharpening 40, Masking 30, both noise reductions, every per-pixel stage",
            with(&per_pixel_stages, &|p| {
                p.sharpen_amount = 40.0;
                p.sharpen_masking = 30.0;
                p.noise_luminance = 30.0;
                p.noise_color = 25.0;
            }),
        ),
        (
            "Detail, Clarity, Texture, Dehaze, Shadows, Highlights, every per-pixel stage",
            with(&per_pixel_stages, &|p| {
                p.sharpen_amount = 40.0;
                p.sharpen_masking = 30.0;
                p.noise_luminance = 30.0;
                p.noise_color = 25.0;
                p.clarity = 50.0;
                p.texture = 50.0;
                p.dehaze = 40.0;
                p.shadows = 50.0;
                p.highlights = -50.0;
            }),
        ),
    ];
    for (name, (p, inputs)) in &sets {
        let toned = spatial(p);
        let cpu = time(if toned { runs.min(10) } else { runs }, || {
            let mut px = src.clone();
            develop(&mut px, w, h, p, inputs, false);
        });
        let points = white_black_points(&src, w, h, p, inputs);
        let stages = per_pixel(p, inputs, w, h, false, points).ok_or("not developed on the GPU")?;
        let size = [w as u32, h as u32];
        let upload = try_time(runs, || pass.develop(&device, &queue, None, &src, None, size, &stages).map(drop))?;
        let cached = try_time(runs, || pass.develop(&device, &queue, None, &src, Some(1), size, &stages).map(drop))?;
        println!("{name}:");
        println!("  CPU develop                          {cpu:7.2} ms");
        println!("  GPU, source uploaded every time      {upload:7.2} ms");
        println!("  GPU, source cached (slider drag)     {cached:7.2} ms");
        if !spatial_tone(p) && (p.whites != 0.0 || p.blacks != 0.0) {
            let points_ms = time(runs, || {
                std::hint::black_box(white_black_points(&src, w, h, p, inputs));
            });
            println!("  Whites/Blacks points on the CPU      {points_ms:7.2} ms (cached per WB/Exposure/Contrast in the dialog)");
        }
    }
    Ok(())
}
