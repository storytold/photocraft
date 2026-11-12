//! Camera Raw's GPU preview pass against the CPU develop (the oracle), stage by stage, at 8, 16
//! and 32 bits. Skips, with a message, when there is no adapter or device or it can't render
//! `Rgba32Float`; with `WGPU_BACKEND` set those fail instead, as in `parity.rs`. A pipeline that
//! doesn't build on an adapter that can run it fails. The shader itself is validated without a
//! GPU ([`the_shader_validates_without_a_gpu`]), so a WGSL error fails on every runner.

use photocraft_algo::camera_raw::{CameraRaw, Wheel, develop, per_pixel, white_black_points};
use photocraft_gpu::{CameraRawDevelop, DeviceHealth, Fault};

/// Largest difference from the CPU develop, per channel. The pass evaluates the same
/// expressions in f32, so what remains is the GPU's own `pow`, `exp2` and `sin` (a few ulps) and
/// how they move a value across a table entry; measured at most about 1e-5 on NVIDIA (Vulkan,
/// DX12) and WARP. 1/4096 keeps a margin and stays well under one 8-bit step (the compositor's
/// tolerance is 1/255).
const TOL: f32 = 1.0 / 4096.0;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    dev: CameraRawDevelop,
    _lock: std::sync::MutexGuard<'static, ()>,
}

fn gpu() -> Option<Gpu> {
    let lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let required = std::env::var_os("WGPU_BACKEND").is_some();
    // `WGPU_BACKEND` picks the backend; `PHOTOCRAFT_TEST_FALLBACK_ADAPTER=1` the software one (WARP).
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let fallback = std::env::var_os("PHOTOCRAFT_TEST_FALLBACK_ADAPTER").is_some();
    let options = wgpu::RequestAdapterOptions { force_fallback_adapter: fallback, ..Default::default() };
    let adapter = match pollster::block_on(instance.request_adapter(&options)) {
        Ok(a) => a,
        Err(e) => {
            assert!(!required, "WGPU_BACKEND is set but has no adapter: {e}");
            eprintln!("skipping Camera Raw GPU tests: no adapter ({e})");
            return None;
        }
    };
    let info = adapter.get_info();
    if !CameraRawDevelop::supported(&adapter) {
        assert!(!required, "WGPU_BACKEND is set but {} can't render Rgba32Float", info.name);
        eprintln!("skipping Camera Raw GPU tests: adapter can't render Rgba32Float");
        return None;
    }
    let (device, queue) = match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) {
        Ok(d) => d,
        Err(e) => {
            assert!(!required, "WGPU_BACKEND is set but {} has no device: {e}", info.name);
            eprintln!("skipping Camera Raw GPU tests: no device ({e})");
            return None;
        }
    };
    eprintln!("Camera Raw GPU tests on {:?} ({})", info.backend, info.name);
    // The adapter can run the pass: a pipeline that doesn't build is a bug in it, not a skip.
    let dev = CameraRawDevelop::new(&device).unwrap_or_else(|e| panic!("the Camera Raw pipeline didn't build on {}: {}", info.name, e.0));
    Some(Gpu { device, queue, dev, _lock: lock })
}

/// The Camera Raw shaders parse and validate (naga, as wgpu does before any driver sees them),
/// with no adapter: the GPU tests below skip on runners without one. The filters are compiled
/// appended to the tone shader, as the pass builds them.
#[test]
fn the_shaders_validate_without_a_gpu() {
    let tone = format!("{}\n{}", photocraft_gpu::CAMERA_RAW_TONE_SHADER, photocraft_gpu::CAMERA_RAW_FILTERS_SHADER);
    for src in [photocraft_gpu::CAMERA_RAW_SHADER, tone.as_str(), photocraft_gpu::CAMERA_RAW_VIEW_SHADER] {
        let module = wgpu::naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        assert!(!module.entry_points.is_empty());
    }
}

#[derive(Clone, Copy, Debug)]
enum Depth {
    Eight,
    Sixteen,
    Float,
}

const DEPTHS: [Depth; 3] = [Depth::Eight, Depth::Sixteen, Depth::Float];

fn hash(i: usize, c: u64) -> f32 {
    let mut z = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ c.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z = (z ^ (z >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// Gradients, noise and saturated patches (every Color Mixer band), quantized to `depth`. The
/// 32-bit image also has values above 1 and a few below 0.
fn image(w: usize, h: usize, depth: Depth) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
            let mut c = if (i / w).is_multiple_of(4) {
                // A hue sweep at full and half saturation.
                let hue = x * 6.0;
                let f = |n: f32| (((hue + n) % 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
                let s = if (i / w).is_multiple_of(8) { 1.0 } else { 0.5 };
                [f(0.0), f(4.0), f(2.0)].map(|v| (1.0 - s) * 0.6 + s * v)
            } else {
                [0.1 + 0.8 * x, 0.2 + 0.6 * y, 0.5 - 0.4 * x * y].map(|v| v + (hash(i, 1) - 0.5) * 0.15)
            };
            if let Depth::Float = depth {
                c = c.map(|v| v * (1.0 + 1.5 * y) - 0.02 * hash(i, 2));
            }
            let q = |v: f32| match depth {
                Depth::Eight => (v.clamp(0.0, 1.0) * 255.0).round() / 255.0,
                Depth::Sixteen => (v.clamp(0.0, 1.0) * 32768.0).round() / 32768.0,
                Depth::Float => v,
            };
            [q(c[0]), q(c[1]), q(c[2]), 1.0]
        })
        .collect()
}

/// Develops `p` both ways and returns the largest channel difference.
fn compare(g: &mut Gpu, p: &CameraRaw, depth: Depth) -> f32 {
    compare_at(g, p, depth, (171, 113))
}

fn compare_at(g: &mut Gpu, p: &CameraRaw, depth: Depth, (w, h): (usize, usize)) -> f32 {
    compare_image(g, p, depth, &image(w, h, depth), (w, h))
}

fn compare_image(g: &mut Gpu, p: &CameraRaw, depth: Depth, src: &[[f32; 4]], (w, h): (usize, usize)) -> f32 {
    let float = matches!(depth, Depth::Float);
    let src = src.to_vec();
    let mut cpu = src.clone();
    develop(&mut cpu, w, h, p, float);
    let points = white_black_points(&src, w, h, p);
    let stages = per_pixel(p, w, h, float, points).unwrap_or_else(|| panic!("{p:?} has a spatial stage"));
    let out = g.dev.develop(&g.device, &g.queue, None, &src, None, [w as u32, h as u32], &stages).unwrap();
    assert_eq!(out.len(), cpu.len());
    let mut worst = (0.0f32, 0usize);
    for (i, (a, b)) in out.iter().zip(&cpu).enumerate() {
        assert!(a.iter().all(|v| v.is_finite()), "{p:?} {depth:?}: non-finite GPU pixel {i}: {a:?}");
        assert_eq!(a[3], b[3], "alpha is kept");
        for c in 0..3 {
            let d = (a[c] - b[c]).abs();
            if d > worst.0 {
                worst = (d, i);
            }
        }
    }
    let (d, i) = worst;
    assert!(d <= TOL, "{p:?} {depth:?}: GPU {:?} vs CPU {:?} at pixel {i} (source {:?}), off by {d}", out[i], cpu[i], src[i]);
    if !float {
        assert!(out.iter().all(|q| q[..3].iter().all(|v| (0.0..=1.0).contains(v))), "{p:?} {depth:?}: clipped like the CPU");
    }
    d
}

fn check(name: &str, sets: &[CameraRaw]) {
    let Some(mut g) = gpu() else { return };
    for p in sets {
        for depth in DEPTHS {
            let d = compare(&mut g, p, depth);
            eprintln!("{name} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
}

#[test]
fn white_balance_and_exposure() {
    check(
        "white balance",
        &[
            CameraRaw { temperature: 40.0, tint: -25.0, ..Default::default() },
            CameraRaw { temperature: -70.0, tint: 60.0, exposure: -1.2, ..Default::default() },
            CameraRaw { exposure: 1.5, ..Default::default() },
        ],
    );
}

#[test]
fn contrast_without_texture_or_clarity() {
    check("contrast", &[CameraRaw { contrast: 60.0, ..Default::default() }, CameraRaw { contrast: -80.0, exposure: 0.4, ..Default::default() }]);
}

#[test]
fn whites_and_blacks() {
    check(
        "whites/blacks",
        &[
            CameraRaw { whites: 70.0, blacks: -45.0, ..Default::default() },
            CameraRaw { whites: -60.0, blacks: 50.0, temperature: 20.0, ..Default::default() },
            CameraRaw { whites: 30.0, ..Default::default() },
            CameraRaw { blacks: -100.0, exposure: -0.5, ..Default::default() },
            // ProPhoto's power-law encoding.
            CameraRaw { whites: 50.0, blacks: -30.0, encoding_gamma: 1.8, ..Default::default() },
        ],
    );
}

#[test]
fn tone_curves() {
    check(
        "tone curves",
        &[
            CameraRaw {
                curve_highlights: -60.0,
                curve_lights: 30.0,
                curve_darks: 40.0,
                curve_shadows: -20.0,
                curve_splits: [20.0, 45.0, 80.0],
                ..Default::default()
            },
            CameraRaw { point_curve: vec![[0.0, 20.0], [64.0, 50.0], [192.0, 220.0], [255.0, 245.0]], ..Default::default() },
            CameraRaw {
                point_curve_red: vec![[0.0, 0.0], [128.0, 160.0], [255.0, 255.0]],
                point_curve_green: vec![[0.0, 10.0], [255.0, 230.0]],
                point_curve_blue: vec![[0.0, 0.0], [100.0, 70.0], [255.0, 255.0]],
                ..Default::default()
            },
        ],
    );
}

#[test]
fn vibrance_and_saturation() {
    check(
        "presence",
        &[
            CameraRaw { vibrance: 70.0, ..Default::default() },
            CameraRaw { saturation: -60.0, ..Default::default() },
            CameraRaw { vibrance: -40.0, saturation: 50.0, ..Default::default() },
        ],
    );
}

#[test]
fn color_mixer() {
    let ramp = |k: f32| std::array::from_fn(|i| ((i as f32 * 37.0 + k) % 200.0) - 100.0);
    check(
        "color mixer",
        &[
            CameraRaw { hsl_hue: ramp(0.0), ..Default::default() },
            CameraRaw { hsl_sat: ramp(50.0), ..Default::default() },
            CameraRaw { hsl_lum: ramp(120.0), ..Default::default() },
            CameraRaw { hsl_hue: ramp(10.0), hsl_sat: ramp(80.0), hsl_lum: ramp(150.0), ..Default::default() },
        ],
    );
}

#[test]
fn color_grading() {
    check(
        "color grading",
        &[
            CameraRaw { grade_shadows: Wheel { hue: 220.0, sat: 70.0, lum: 20.0 }, ..Default::default() },
            CameraRaw {
                grade_midtones: Wheel { hue: 120.0, sat: 30.0, lum: -15.0 },
                grade_highlights: Wheel { hue: 40.0, sat: 50.0, lum: 10.0 },
                grade_global: Wheel { hue: 300.0, sat: 15.0, lum: 5.0 },
                grade_blending: 80.0,
                grade_balance: -40.0,
                ..Default::default()
            },
        ],
    );
}

#[test]
fn vignette() {
    check(
        "vignette",
        &[
            CameraRaw { vignette_amount: -70.0, ..Default::default() },
            CameraRaw { vignette_amount: -50.0, vignette_roundness: -60.0, vignette_feather: 20.0, vignette_highlights: 70.0, ..Default::default() },
            CameraRaw {
                vignette_amount: 60.0,
                vignette_roundness: 80.0,
                vignette_midpoint: 20.0,
                vignette_style: "colorPriority".into(),
                ..Default::default()
            },
            CameraRaw { vignette_amount: -40.0, vignette_style: "paintOverlay".into(), ..Default::default() },
            CameraRaw { vignette_amount: 40.0, vignette_style: "paintOverlay".into(), ..Default::default() },
        ],
    );
}

#[test]
fn grain_in_full_resolution_coordinates() {
    check(
        "grain",
        &[
            CameraRaw { grain_amount: 60.0, seed: 3, ..Default::default() },
            // A preview proxy: the noise lattice is sampled at full-resolution positions.
            CameraRaw { grain_amount: 80.0, grain_size: 60.0, grain_roughness: 90.0, seed: u32::MAX, pixel_scale: 0.143, ..Default::default() },
            CameraRaw { grain_amount: 25.0, grain_size: 0.0, grain_roughness: 0.0, seed: 77, pixel_scale: 0.5, ..Default::default() },
        ],
    );
}

#[test]
fn every_per_pixel_stage_at_once() {
    let mut hsl = [0.0; 8];
    hsl[1] = 35.0;
    hsl[5] = -50.0;
    check(
        "all stages",
        &[CameraRaw {
            temperature: 25.0,
            tint: 10.0,
            exposure: 0.3,
            contrast: 25.0,
            whites: 20.0,
            blacks: -15.0,
            vibrance: 30.0,
            saturation: 10.0,
            curve_darks: 20.0,
            point_curve: vec![[0.0, 8.0], [255.0, 250.0]],
            hsl_hue: hsl,
            hsl_sat: hsl,
            hsl_lum: hsl,
            grade_shadows: Wheel { hue: 210.0, sat: 25.0, lum: 0.0 },
            vignette_amount: -30.0,
            grain_amount: 20.0,
            pixel_scale: 0.2,
            ..Default::default()
        }],
    );
}

#[test]
fn identity_leaves_pixels_alone_and_detail_has_stages() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw::default();
    let src = image(64, 32, Depth::Float);
    let stages = per_pixel(&p, 64, 32, false, None).unwrap();
    let out = g.dev.develop(&g.device, &g.queue, None, &src, Some(1), [64, 32], &stages).unwrap();
    assert_eq!(out, src, "the CPU develop returns early, unclipped");
    for spatial in [
        CameraRaw { sharpen_amount: 10.0, ..Default::default() },
        CameraRaw { noise_luminance: 10.0, clarity: 20.0, ..Default::default() },
        CameraRaw { noise_color: 10.0, dehaze: 20.0, ..Default::default() },
    ] {
        assert!(spatial.spatial() && per_pixel(&spatial, 64, 32, false, Some((0.0, 1.0))).is_some_and(|s| s.detail.is_some()), "{spatial:?}");
    }
    // Whites and Blacks need the image's points, unless a spatial stage's passes measure them.
    // Detail comes after them, so with only Detail the points are measured on the CPU.
    assert!(per_pixel(&CameraRaw { whites: 10.0, ..Default::default() }, 64, 32, false, None).is_none());
    let sharpened = CameraRaw { whites: 10.0, sharpen_amount: 30.0, ..Default::default() };
    assert!(per_pixel(&sharpened, 64, 32, false, None).is_none());
    let points = white_black_points(&src, 64, 32, &sharpened);
    let s = per_pixel(&sharpened, 64, 32, false, points).unwrap();
    assert!(s.whites_blacks.is_some() && s.measure_whites_blacks.is_none() && s.detail.is_some());
    let toned = per_pixel(&CameraRaw { whites: 10.0, shadows: 20.0, ..Default::default() }, 64, 32, false, None).unwrap();
    assert!(toned.local_tone.is_some() && toned.whites_blacks.is_none() && toned.measure_whites_blacks == Some([10.0, 0.0]));
    for p in [
        CameraRaw { blacks: -10.0, dehaze: 20.0, ..Default::default() },
        CameraRaw { blacks: -10.0, clarity: 20.0, ..Default::default() },
        CameraRaw { blacks: -10.0, texture: 20.0, ..Default::default() },
    ] {
        let s = per_pixel(&p, 64, 32, false, None).unwrap();
        assert!(s.whites_blacks.is_none() && s.measure_whites_blacks == Some([0.0, -10.0]), "{p:?}");
    }
    // Wider than the filter's working copy, the develop filters a reduced copy: the CPU's.
    assert!(per_pixel(&CameraRaw { shadows: 20.0, ..Default::default() }, 1601, 32, false, None).is_none());
    // At 2 pixels on the short side the pass's pyramid has one level: the CPU's too.
    for (w, h) in [(2, 2), (40, 2), (2, 40)] {
        assert!(per_pixel(&CameraRaw { highlights: -20.0, ..Default::default() }, w, h, false, None).is_none(), "{w}×{h}");
    }
}

#[test]
fn a_cached_source_is_reused_and_a_new_key_uploads() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw { exposure: 0.5, ..Default::default() };
    let stages = per_pixel(&p, 64, 32, false, None).unwrap();
    let a = image(64, 32, Depth::Eight);
    let b: Vec<[f32; 4]> = a.iter().map(|q| [q[2], q[0], q[1], 1.0]).collect();
    let first = g.dev.develop(&g.device, &g.queue, None, &a, Some(7), [64, 32], &stages).unwrap();
    // Same key: the uploaded source stays (callers key by the proxy's identity).
    assert_eq!(g.dev.develop(&g.device, &g.queue, None, &b, Some(7), [64, 32], &stages).unwrap(), first);
    let mut cpu = b.clone();
    develop(&mut cpu, 64, 32, &p, false);
    let other = g.dev.develop(&g.device, &g.queue, None, &b, Some(8), [64, 32], &stages).unwrap();
    assert!(other.iter().zip(&cpu).all(|(x, y)| (0..3).all(|c| (x[c] - y[c]).abs() <= TOL)));
}

#[test]
fn bad_sizes_and_faulted_devices_are_errors() {
    let Some(mut g) = gpu() else { return };
    let stages = per_pixel(&CameraRaw { exposure: 0.5, ..Default::default() }, 4, 4, false, None).unwrap();
    let px = vec![[0.5f32; 4]; 16];
    for size in [[0, 4], [4, 0], [5, 4], [3, 3], [u32::MAX, 1]] {
        assert!(g.dev.develop(&g.device, &g.queue, None, &px, None, size, &stages).is_err(), "{size:?}");
    }
    let health = DeviceHealth::new();
    health.mark(Fault::Lost("test".into()));
    assert!(g.dev.develop(&g.device, &g.queue, Some(&health), &px, None, [4, 4], &stages).is_err());
    // And the pass still works afterwards.
    assert!(g.dev.develop(&g.device, &g.queue, Some(&DeviceHealth::new()), &px, None, [4, 4], &stages).is_ok());
}

#[test]
fn shadows_and_highlights() {
    let sets = [
        CameraRaw { shadows: 50.0, ..Default::default() },
        CameraRaw { shadows: -70.0, ..Default::default() },
        CameraRaw { highlights: -50.0, ..Default::default() },
        CameraRaw { highlights: 60.0, ..Default::default() },
        CameraRaw { shadows: 100.0, highlights: -100.0, ..Default::default() },
        CameraRaw { shadows: 35.0, highlights: -45.0, encoding_gamma: 1.8, ..Default::default() },
    ];
    assert_moves(&sets);
    check("shadows/highlights", &sets);
}

#[test]
fn shadows_and_highlights_on_tiny_odd_and_flat_frames() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw { shadows: 60.0, highlights: -40.0, whites: 20.0, ..Default::default() };
    // A frame 2 pixels on a side has a one-level pyramid: `per_pixel` sends it to the CPU, and
    // the pass still refuses stages built for it elsewhere.
    for (w, h) in [(2, 2), (40, 2)] {
        assert!(per_pixel(&p, w, h, false, None).is_none(), "{w}×{h}");
        let stages = per_pixel(&p, 3, 3, false, None).unwrap();
        assert!(g.dev.develop(&g.device, &g.queue, None, &image(w, h, Depth::Eight), None, [w as u32, h as u32], &stages).is_err());
    }
    for size in [(3, 3), (5, 3), (37, 23), (300, 7)] {
        for depth in DEPTHS {
            let d = compare_at(&mut g, &p, depth, size);
            eprintln!("shadows/highlights {size:?} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
    // A flat frame has no range to filter.
    let flat = vec![[0.4, 0.35, 0.3, 1.0]; 64 * 48];
    for depth in DEPTHS {
        compare_image(&mut g, &p, depth, &flat, (64, 48));
    }
}

#[test]
fn shadows_and_highlights_with_per_pixel_stages() {
    let mut hsl = [0.0; 8];
    hsl[1] = 35.0;
    hsl[5] = -50.0;
    let all = CameraRaw {
        temperature: 25.0,
        tint: 10.0,
        exposure: 0.3,
        contrast: 25.0,
        shadows: 45.0,
        highlights: -55.0,
        whites: 20.0,
        blacks: -15.0,
        vibrance: 30.0,
        saturation: 10.0,
        curve_darks: 20.0,
        point_curve: vec![[0.0, 8.0], [255.0, 250.0]],
        hsl_hue: hsl,
        hsl_sat: hsl,
        hsl_lum: hsl,
        grade_shadows: Wheel { hue: 210.0, sat: 25.0, lum: 0.0 },
        vignette_amount: -30.0,
        grain_amount: 20.0,
        pixel_scale: 0.2,
        ..Default::default()
    };
    check(
        "shadows/highlights + per pixel",
        &[
            all.clone(),
            // Whites and Blacks measured after the filter, both directions.
            CameraRaw { shadows: 60.0, whites: 70.0, blacks: -45.0, ..Default::default() },
            CameraRaw { highlights: -60.0, whites: -60.0, blacks: 50.0, exposure: -0.4, ..Default::default() },
            CameraRaw { shadows: 30.0, highlights: -30.0, whites: 40.0, blacks: -30.0, encoding_gamma: 1.8, ..Default::default() },
        ],
    );
    // At the dialog's proxy size.
    let Some(mut g) = gpu() else { return };
    for depth in DEPTHS {
        let d = compare_at(&mut g, &all, depth, (858, 572));
        eprintln!("shadows/highlights + per pixel 858x572 {depth:?}: max |GPU - CPU| = {d:.2e}");
    }
}

/// Asserts that each of `sets` moves some pixel of the test image by over 2 %, so a comparison
/// with the CPU can't pass trivially.
fn assert_moves(sets: &[CameraRaw]) {
    for p in sets {
        let src = image(171, 113, Depth::Sixteen);
        let mut out = src.clone();
        develop(&mut out, 171, 113, p, false);
        let moved = out.iter().zip(&src).flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
        assert!(moved > 0.02, "{p:?} moves pixels by at most {moved}");
    }
}

#[test]
fn clarity_texture_and_dehaze() {
    let sets = [
        CameraRaw { clarity: 50.0, ..Default::default() },
        CameraRaw { clarity: -80.0, ..Default::default() },
        CameraRaw { texture: 50.0, ..Default::default() },
        CameraRaw { texture: -70.0, pixel_scale: 0.143, ..Default::default() },
        CameraRaw { dehaze: 40.0, ..Default::default() },
        CameraRaw { dehaze: 100.0, ..Default::default() },
        CameraRaw { dehaze: -60.0, ..Default::default() },
        // With Contrast, which then runs after them on the same luminance.
        CameraRaw { clarity: 40.0, texture: 30.0, contrast: -35.0, ..Default::default() },
        CameraRaw { dehaze: 30.0, clarity: 45.0, texture: -40.0, contrast: 20.0, ..Default::default() },
        CameraRaw { dehaze: -30.0, clarity: -25.0, texture: 60.0, encoding_gamma: 1.8, ..Default::default() },
    ];
    assert_moves(&sets);
    check("clarity/texture/dehaze", &sets);
}

#[test]
fn clarity_texture_and_dehaze_with_shadows_highlights_and_per_pixel_stages() {
    let mut hsl = [0.0; 8];
    hsl[1] = 35.0;
    hsl[5] = -50.0;
    let all = CameraRaw {
        temperature: 25.0,
        tint: 10.0,
        exposure: 0.3,
        contrast: 25.0,
        dehaze: 35.0,
        shadows: 45.0,
        highlights: -55.0,
        clarity: 40.0,
        texture: 30.0,
        whites: 20.0,
        blacks: -15.0,
        vibrance: 30.0,
        saturation: 10.0,
        curve_darks: 20.0,
        point_curve: vec![[0.0, 8.0], [255.0, 250.0]],
        hsl_hue: hsl,
        hsl_sat: hsl,
        hsl_lum: hsl,
        grade_shadows: Wheel { hue: 210.0, sat: 25.0, lum: 0.0 },
        vignette_amount: -30.0,
        grain_amount: 20.0,
        pixel_scale: 0.2,
        ..Default::default()
    };
    let sets = [
        all.clone(),
        CameraRaw { dehaze: 50.0, shadows: 60.0, highlights: -40.0, ..Default::default() },
        CameraRaw { clarity: 60.0, texture: 50.0, shadows: 40.0, ..Default::default() },
        // Whites and Blacks measured after them, without Shadows and Highlights, both directions.
        CameraRaw { dehaze: 40.0, whites: 60.0, blacks: -40.0, ..Default::default() },
        CameraRaw { clarity: 50.0, whites: -50.0, blacks: 45.0, exposure: -0.3, ..Default::default() },
        CameraRaw { texture: -60.0, whites: 30.0, blacks: -60.0, ..Default::default() },
    ];
    check("clarity/texture/dehaze + shadows/highlights + per pixel", &sets);
    let Some(mut g) = gpu() else { return };
    // At the dialog's proxy size (Dehaze and Clarity on a copy subsampled twice), and wider (three
    // times).
    for size in [(858, 572), (1600, 900)] {
        for depth in DEPTHS {
            let d = compare_at(&mut g, &all, depth, size);
            eprintln!("everything {size:?} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
}

#[test]
fn clarity_texture_and_dehaze_on_tiny_and_odd_frames() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw { dehaze: 45.0, clarity: 50.0, texture: 60.0, whites: 20.0, blacks: -20.0, ..Default::default() };
    // Without Shadows and Highlights even 1 and 2 pixel frames develop on the GPU.
    for size in [(1, 1), (2, 2), (40, 2), (3, 3), (5, 3), (37, 23), (300, 7)] {
        for depth in DEPTHS {
            let d = compare_at(&mut g, &p, depth, size);
            eprintln!("clarity/texture/dehaze {size:?} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
    // Texture's blur on a small proxy is too narrow to change anything; the stage still runs.
    let narrow = CameraRaw { texture: 80.0, contrast: 10.0, pixel_scale: 0.1, ..Default::default() };
    assert!(per_pixel(&narrow, 64, 48, true, None).and_then(|s| s.texture).is_some_and(|t| t.radii.is_empty()));
    for depth in DEPTHS {
        compare_at(&mut g, &narrow, depth, (64, 48));
    }
    // A flat frame: no haze to find, no detail to raise.
    let flat = vec![[0.4, 0.35, 0.3, 1.0]; 64 * 48];
    for depth in DEPTHS {
        compare_image(&mut g, &p, depth, &flat, (64, 48));
    }
}

/// #9273: the GPU opens the dark channel like the CPU, so a sky beside a dark tower clears as it
/// does there. The scene of algo's `dehaze_leaves_no_halo_beside_dark_objects`: a clear sky and a
/// dark tower hazed with `I = J·t + A·(1 − t)`.
#[test]
fn dehaze_opens_the_dark_channel_like_the_cpu() {
    let Some(mut g) = gpu() else { return };
    let (w, h) = (600usize, 400usize);
    let tower = |x: usize, y: usize| (250..350).contains(&x) && y >= 120;
    let (t, air) = (0.55f32, [0.9f32, 0.9, 0.9]);
    let src: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let j = if tower(x, y) { [0.06, 0.06, 0.07] } else { [0.35 + 0.1 * y as f32 / h as f32, 0.5, 0.8] };
            let c: [f32; 3] = std::array::from_fn(|k| j[k] * t + air[k] * (1.0 - t));
            [c[0], c[1], c[2], 1.0]
        })
        .collect();
    for dehaze in [100.0, 60.0, -40.0] {
        for depth in DEPTHS {
            let d = compare_image(&mut g, &CameraRaw { dehaze, ..Default::default() }, depth, &src, (w, h));
            eprintln!("dehaze {dehaze} on the tower {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
}

/// Each of Detail's controls alone and together: sharpening (Detail at 0 and 100, Masking off
/// and on), luminance and colour noise reduction (their Detail at 0 and 100), at full size and on
/// a proxy.
#[test]
fn detail_sharpening_and_noise_reduction() {
    let sets = [
        CameraRaw { sharpen_amount: 60.0, ..Default::default() },
        CameraRaw { sharpen_amount: 100.0, sharpen_radius: 3.0, sharpen_detail: 100.0, ..Default::default() },
        // Detail at 0 limits each change to 1.5 % of Amount's strength: 150 still moves over 2 %.
        CameraRaw { sharpen_amount: 150.0, sharpen_radius: 0.5, sharpen_detail: 0.0, ..Default::default() },
        CameraRaw { sharpen_amount: 80.0, sharpen_radius: 1.5, sharpen_masking: 60.0, ..Default::default() },
        CameraRaw { sharpen_amount: 120.0, sharpen_detail: 70.0, sharpen_masking: 100.0, ..Default::default() },
        CameraRaw { sharpen_amount: 100.0, sharpen_masking: 15.0, ..Default::default() },
        CameraRaw { noise_luminance: 50.0, ..Default::default() },
        CameraRaw { noise_luminance: 100.0, noise_luminance_detail: 0.0, ..Default::default() },
        CameraRaw { noise_luminance: 80.0, noise_luminance_detail: 100.0, ..Default::default() },
        CameraRaw { noise_color: 60.0, ..Default::default() },
        CameraRaw { noise_color: 100.0, noise_color_detail: 0.0, ..Default::default() },
        CameraRaw { noise_color: 30.0, noise_color_detail: 100.0, ..Default::default() },
        CameraRaw { noise_luminance: 40.0, noise_color: 50.0, sharpen_amount: 70.0, ..Default::default() },
        CameraRaw { noise_luminance: 40.0, noise_color: 50.0, sharpen_amount: 90.0, sharpen_masking: 40.0, sharpen_detail: 60.0, ..Default::default() },
        // A proxy: the radii scale with the pixel scale (sharpening's and its mask's at their floor).
        CameraRaw { noise_luminance: 60.0, noise_color: 70.0, sharpen_amount: 100.0, sharpen_masking: 30.0, pixel_scale: 0.143, ..Default::default() },
        CameraRaw { sharpen_amount: 120.0, sharpen_radius: 3.0, encoding_gamma: 1.8, pixel_scale: 0.5, ..Default::default() },
    ];
    assert_moves(&sets);
    check("detail", &sets);
}

#[test]
fn detail_with_every_other_stage() {
    let mut hsl = [0.0; 8];
    hsl[1] = 35.0;
    hsl[5] = -50.0;
    let all = CameraRaw {
        temperature: 25.0,
        tint: 10.0,
        exposure: 0.3,
        contrast: 25.0,
        dehaze: 35.0,
        shadows: 45.0,
        highlights: -55.0,
        clarity: 40.0,
        texture: 30.0,
        whites: 20.0,
        blacks: -15.0,
        vibrance: 30.0,
        saturation: 10.0,
        curve_darks: 20.0,
        point_curve: vec![[0.0, 8.0], [255.0, 250.0]],
        hsl_hue: hsl,
        hsl_sat: hsl,
        hsl_lum: hsl,
        grade_shadows: Wheel { hue: 210.0, sat: 25.0, lum: 0.0 },
        sharpen_amount: 80.0,
        sharpen_masking: 25.0,
        noise_luminance: 30.0,
        noise_color: 40.0,
        vignette_amount: -30.0,
        grain_amount: 20.0,
        pixel_scale: 0.2,
        ..Default::default()
    };
    let sets = [
        all.clone(),
        // Whites and Blacks measured on the CPU (Detail, after them, is the only spatial stage),
        CameraRaw { whites: 60.0, blacks: -40.0, contrast: 20.0, sharpen_amount: 90.0, noise_luminance: 30.0, ..Default::default() },
        CameraRaw { whites: -50.0, blacks: 45.0, exposure: -0.3, noise_color: 60.0, ..Default::default() },
        // and on the GPU after Clarity.
        CameraRaw { clarity: 50.0, whites: 30.0, blacks: -60.0, sharpen_amount: 70.0, sharpen_masking: 50.0, ..Default::default() },
        CameraRaw {
            shadows: 50.0,
            vignette_amount: 40.0,
            vignette_style: "paintOverlay".into(),
            noise_color: 50.0,
            sharpen_amount: 60.0,
            ..Default::default()
        },
    ];
    check("detail + every stage", &sets);
    let Some(mut g) = gpu() else { return };
    // At the dialog's proxy size, with and without the other stages.
    let proxy = CameraRaw { noise_luminance: 50.0, noise_color: 50.0, sharpen_amount: 100.0, sharpen_masking: 20.0, pixel_scale: 0.143, ..Default::default() };
    for p in [CameraRaw { pixel_scale: 0.143, ..all.clone() }, proxy] {
        for depth in DEPTHS {
            let d = compare_at(&mut g, &p, depth, (858, 572));
            eprintln!("detail 858x572 {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
}

#[test]
fn detail_on_tiny_odd_and_flat_frames() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw { noise_luminance: 60.0, noise_color: 50.0, sharpen_amount: 100.0, sharpen_masking: 30.0, whites: 20.0, ..Default::default() };
    for size in [(1, 1), (2, 2), (40, 2), (2, 40), (3, 3), (5, 3), (37, 23), (300, 7)] {
        for depth in DEPTHS {
            let d = compare_at(&mut g, &p, depth, size);
            eprintln!("detail {size:?} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
    // A flat frame: nothing to reduce or sharpen, no edges for the mask.
    let flat = vec![[0.4, 0.35, 0.3, 1.0]; 64 * 48];
    for depth in DEPTHS {
        compare_image(&mut g, &p, depth, &flat, (64, 48));
    }
}

/// Settings that change which of Detail's parts run, in turn on one pass: each develop picks its
/// own plan and textures and still matches the CPU.
#[test]
fn detail_parts_switch_on_and_off_between_develops() {
    let Some(mut g) = gpu() else { return };
    let sets = [
        CameraRaw { noise_color: 50.0, ..Default::default() },
        CameraRaw { sharpen_amount: 80.0, sharpen_masking: 40.0, ..Default::default() },
        CameraRaw { exposure: 0.3, ..Default::default() },
        CameraRaw { noise_luminance: 50.0, sharpen_amount: 80.0, ..Default::default() },
        CameraRaw { noise_color: 50.0, ..Default::default() },
    ];
    for p in &sets {
        for depth in DEPTHS {
            compare_at(&mut g, p, depth, (171, 113));
        }
    }
}

#[test]
fn dehaze_remakes_its_copy_when_the_white_balance_changes() {
    let Some(mut g) = gpu() else { return };
    let src = image(858, 572, Depth::Eight);
    // Same source key throughout: the copy is cached by it and by the white balance.
    for temperature in [25.0, -40.0, -40.0, 0.0] {
        let p = CameraRaw { dehaze: 50.0, temperature, ..Default::default() };
        let mut cpu = src.clone();
        develop(&mut cpu, 858, 572, &p, false);
        let stages = per_pixel(&p, 858, 572, false, None).unwrap();
        let out = g.dev.develop(&g.device, &g.queue, None, &src, Some(3), [858, 572], &stages).unwrap();
        let d = out.iter().zip(&cpu).flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
        assert!(d <= TOL, "temperature {temperature}: off by {d}");
    }
}

// The preview shown from the GPU (`develop_view`): its pixels, display texture and histogram.

/// Reads an `Rgba8Unorm` texture back.
fn read_bytes(g: &Gpu, texture: &wgpu::Texture) -> Vec<[u8; 4]> {
    let (w, h) = (texture.width(), texture.height());
    let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = g.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = g.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let index = g.queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    g.device.poll(wgpu::PollType::Wait { submission_index: Some(index), timeout: None }).unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap();
    (0..h as usize).flat_map(|y| data[y * row as usize..][..w as usize * 4].as_chunks::<4>().0.to_vec()).collect()
}

/// egui's `Color32::from_rgba_unmultiplied`, as bytes.
fn premultiplied([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    let mul = |v: u8| {
        let p = u16::from(v) * u16::from(a) + 128;
        ((p + (p >> 8)) >> 8) as u8
    };
    match a {
        0 => [0; 4],
        255 => [r, g, b, 255],
        _ => [mul(r), mul(g), mul(b), a],
    }
}

/// `CanvasDisplay::to_rgba8`'s 8-bit codes (`photocraft-engine`'s `display_color`): plain, or
/// through the sRGB-encoding table.
fn codes(p: [f32; 4], srgb: bool) -> [u8; 4] {
    let plain = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let encoded = |v: f32| {
        let i = (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
        (photocraft_color::convert::linear_to_srgb(i as f32 / 65535.0) * 255.0 + 0.5) as u8
    };
    if srgb { [encoded(p[0]), encoded(p[1]), encoded(p[2]), plain(p[3])] } else { [plain(p[0]), plain(p[1]), plain(p[2]), plain(p[3])] }
}

/// Develops `p` with `develop_view` and checks it against `develop`: the same pixels, a histogram
/// equal to the CPU's count of them, and display bytes equal to the CPU's conversion of them.
/// `false` when the frame doesn't develop on the GPU (Shadows and Highlights 2 pixels or less on
/// the short side; Whites and Blacks without points).
fn check_view(g: &mut Gpu, p: &CameraRaw, src: &[[f32; 4]], (w, h): (usize, usize), srgb: bool) -> bool {
    let Some(stages) = per_pixel(p, w, h, false, white_black_points(src, w, h, p)) else { return false };
    let size = [w as u32, h as u32];
    let read = g.dev.develop(&g.device, &g.queue, None, src, None, size, &stages).unwrap();
    let display = photocraft_gpu::CameraRawDisplay { encode_srgb: srgb, shaper: None, key: 1 };
    let frame = g.dev.develop_view(&g.device, &g.queue, None, src, None, size, &stages, &display, None, false).unwrap();
    assert_eq!((frame.slot, frame.size), (0, size));
    let px = photocraft_gpu::read_camera_raw_pixels(&g.device, &g.queue, None, &frame.pixels, &mut None).unwrap();
    let same = px.len() == read.len() && px.iter().zip(&read).all(|(a, b)| a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()));
    assert!(same, "{w}×{h} {p:?}: the view's pixels are the develop's");
    // Asked for, the pixels come back with the frame, the same bits.
    let with = g.dev.develop_view(&g.device, &g.queue, None, src, None, size, &stages, &display, Some(frame.slot), true).unwrap();
    let back = with.read_back.as_deref().unwrap_or_default();
    let bits = |v: &[[f32; 4]]| v.iter().flatten().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(back), bits(&px), "{w}×{h} {p:?}: read back with the frame");
    assert_eq!(frame.histogram, photocraft_algo::histogram::RgbHistogram::from_rgba(&px), "{w}×{h} {p:?}");
    let shown = read_bytes(g, &frame.display);
    for (i, (got, q)) in shown.iter().zip(&px).enumerate() {
        assert_eq!(*got, premultiplied(codes(*q, srgb)), "{w}×{h} srgb {srgb}: pixel {i} {q:?}");
    }
    true
}

/// Every stage kind at each depth, plain and sRGB-encoded, at the proxy's size and on tiny and
/// odd frames: exact pixels, histogram and display bytes.
#[test]
fn the_view_shows_and_counts_the_developed_frame() {
    let Some(mut g) = gpu() else { return };
    let sets = [
        CameraRaw::default(),
        CameraRaw { temperature: 20.0, exposure: 0.6, contrast: 30.0, whites: 40.0, blacks: -30.0, vibrance: 40.0, ..Default::default() },
        CameraRaw { shadows: 50.0, highlights: -50.0, clarity: 30.0, sharpen_amount: 60.0, noise_color: 30.0, ..Default::default() },
    ];
    for depth in DEPTHS {
        for (w, h) in [(858, 572), (171, 113), (1, 1), (3, 2)] {
            let src = image(w, h, depth);
            for p in &sets {
                for srgb in [false, true] {
                    let checked = check_view(&mut g, p, &src, (w, h), srgb);
                    assert!(checked || w.min(h) <= 2, "{w}×{h} {p:?} develops on the GPU");
                }
            }
        }
    }
}

/// Transparent, out-of-range and non-finite pixels count where `RgbHistogram` counts them, and
/// show as egui would draw them: the identity develop passes them through.
#[test]
fn the_view_counts_transparent_out_of_range_and_non_finite_pixels() {
    let Some(mut g) = gpu() else { return };
    let (w, h) = (64, 33);
    let mut src = image(w, h, Depth::Float);
    let odd = [
        [0.5, 0.5, 0.5, 0.0],
        [0.2, 0.3, 0.4, 0.5],
        [1.5, -0.25, 1.0, 1.0],
        [0.0, 1.0, 0.0, 0.25],
        [f32::NAN, 0.5, 0.5, 1.0],
        [0.5, f32::INFINITY, 0.5, 1.0],
        [0.5, 0.5, f32::NEG_INFINITY, 1.0],
        [0.5, 0.5, 0.5, f32::NAN],
        [0.5 / 255.0, 1.5 / 255.0, 254.5 / 255.0, 1.0],
    ];
    for (i, p) in src.iter_mut().enumerate() {
        if i % 5 == 0 {
            *p = odd[(i / 5) % odd.len()];
        }
    }
    for srgb in [false, true] {
        assert!(check_view(&mut g, &CameraRaw::default(), &src, (w, h), srgb));
    }
}

/// A matrix/shaper display transform (Display P3 to sRGB) matches `Transform::convert_u8` on the
/// CPU codes within one 8-bit step.
#[test]
fn the_view_applies_a_matrix_shaper_display_transform() {
    use photocraft_cms::{Builtin, Intent, Transform};
    let Some(mut g) = gpu() else { return };
    let t = Transform::new(Builtin::DisplayP3.profile(), Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).unwrap();
    let shaper = t.gpu_matrix_shaper().expect("a matrix/shaper transform");
    let (w, h) = (171, 113);
    let p = CameraRaw { exposure: 0.3, vibrance: 30.0, ..Default::default() };
    let mut worst = 0u8;
    for depth in DEPTHS {
        let mut src = image(w, h, depth);
        src.iter_mut().step_by(7).for_each(|p| p[3] = 0.5);
        let stages = per_pixel(&p, w, h, false, None).unwrap();
        let display = photocraft_gpu::CameraRawDisplay {
            encode_srgb: false,
            shaper: Some(photocraft_gpu::CameraRawShaper { matrix: shaper.matrix_rows(), input: shaper.input_table(), output: shaper.output_table() }),
            key: 7,
        };
        let frame = g.dev.develop_view(&g.device, &g.queue, None, &src, None, [w as u32, h as u32], &stages, &display, None, false).unwrap();
        let px = photocraft_gpu::read_camera_raw_pixels(&g.device, &g.queue, None, &frame.pixels, &mut None).unwrap();
        let src8: Vec<u8> = px.iter().flat_map(|q| codes(*q, false)).collect();
        let mut cpu = src8.clone();
        t.convert_u8(&src8, 4, &mut cpu, 4, true);
        for (got, want) in read_bytes(&g, &frame.display).iter().zip(cpu.as_chunks::<4>().0) {
            let want = premultiplied(*want);
            worst = worst.max(got.iter().zip(&want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0));
        }
    }
    eprintln!("matrix/shaper display: max |GPU - CPU| = {worst} of 255");
    assert!(worst <= 1, "off by {worst} of 255");
}

/// The slots take turns around the one on screen, and each keeps its frame.
#[test]
fn the_view_develops_into_the_slot_not_on_screen() {
    let Some(mut g) = gpu() else { return };
    let (w, h) = (40, 30);
    let src = image(w, h, Depth::Eight);
    let size = [w as u32, h as u32];
    let display = photocraft_gpu::CameraRawDisplay { encode_srgb: false, shaper: None, key: 0 };
    let develop = |g: &mut Gpu, exposure: f32, avoid: Option<usize>| {
        let stages = per_pixel(&CameraRaw { exposure, ..Default::default() }, w, h, false, None).unwrap();
        g.dev.develop_view(&g.device, &g.queue, None, &src, None, size, &stages, &display, avoid, false).unwrap()
    };
    let a = develop(&mut g, -1.0, None);
    let b = develop(&mut g, 1.0, Some(a.slot));
    let c = develop(&mut g, 0.5, Some(b.slot));
    assert_eq!((a.slot, b.slot, c.slot), (0, 1, 0));
    // One buffer for every read, as the dialog keeps it.
    let mut buffer = None;
    let mut read = |f: &photocraft_gpu::CameraRawFrame| photocraft_gpu::read_camera_raw_pixels(&g.device, &g.queue, None, &f.pixels, &mut buffer).unwrap();
    let (rb, rc) = (read(&b), read(&c));
    assert_eq!(read(&b), rb, "a reused buffer reads the frame again");
    assert_ne!(rb, rc, "slot 1 still holds the +1 frame");
    assert_eq!(b.histogram, photocraft_algo::histogram::RgbHistogram::from_rgba(&rb));
    assert_eq!(a.generation, c.generation, "the same size keeps the slot's textures");
}

/// A faulted device, a bad display transform and an impossible size are errors, not panics.
#[test]
fn the_view_rejects_bad_sizes_tables_and_faulted_devices() {
    let Some(mut g) = gpu() else { return };
    let stages = per_pixel(&CameraRaw::default(), 4, 4, false, None).unwrap();
    let display = photocraft_gpu::CameraRawDisplay { encode_srgb: false, shaper: None, key: 0 };
    let src = vec![[0.5; 4]; 16];
    assert!(g.dev.develop_view(&g.device, &g.queue, None, &src, None, [5, 4], &stages, &display, None, false).is_err());
    assert!(g.dev.develop_view(&g.device, &g.queue, None, &src, None, [0, 0], &stages, &display, None, false).is_err());
    let nan = [[f32::NAN; 4]; 3];
    let bad = photocraft_gpu::CameraRawDisplay {
        encode_srgb: false,
        shaper: Some(photocraft_gpu::CameraRawShaper { matrix: nan, input: &[[0.0; 4]; 2], output: &[[1.0; 4]; 2] }),
        key: 9,
    };
    assert!(g.dev.develop_view(&g.device, &g.queue, None, &src, None, [4, 4], &stages, &bad, None, false).is_err());
    let ok = g.dev.develop_view(&g.device, &g.queue, None, &src, None, [4, 4], &stages, &display, None, false).unwrap();
    let health = DeviceHealth::new();
    health.mark(Fault::Lost("test".into()));
    assert!(g.dev.develop_view(&g.device, &g.queue, Some(&health), &src, None, [4, 4], &stages, &display, None, false).is_err());
    assert!(photocraft_gpu::read_camera_raw_pixels(&g.device, &g.queue, Some(&health), &ok.pixels, &mut None).is_err());
}
