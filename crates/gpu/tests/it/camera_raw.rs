//! Camera Raw's GPU preview pass against the CPU develop (the oracle), stage by stage, at an
//! integer depth (8 or 16 bits) and at 32 bits (see [`depths`]). Skips, with a message, when
//! there is no adapter or device or it can't render `Rgba32Float`; with `WGPU_BACKEND` set those
//! fail instead, as in `parity.rs`. A pipeline that doesn't build on an adapter that can run it
//! fails. The shader itself is validated without a GPU ([`the_shader_validates_without_a_gpu`]),
//! so a WGSL error fails on every runner.

use photocraft_algo::camera_raw::{CameraRaw, DevelopInputs, PerPixel, StoredVignetteStyle, TonePoints, Wheel, spatial};

/// `CameraRaw` with the named fields set (the type is `#[non_exhaustive]`, so no struct expression).
macro_rules! cr {
    ($($f:ident : $v:expr),* $(,)?) => {{
        let mut s = CameraRaw::default();
        $(s.$f = $v;)*
        s
    }};
}

/// Settings and the job's inputs (scale, grain seed, encoding): one case for [`check`].
type Case = (CameraRaw, DevelopInputs);

fn case(p: CameraRaw, inputs: DevelopInputs) -> Case {
    (p, inputs)
}

fn develop(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, inputs: &DevelopInputs, float: bool) {
    photocraft_algo::camera_raw::develop(px, w, h, p, inputs, float);
}

/// [`photocraft_algo::camera_raw::per_pixel`] with the default inputs.
fn per_pixel(p: &CameraRaw, w: usize, h: usize, float: bool, points: Option<TonePoints>) -> Option<PerPixel> {
    photocraft_algo::camera_raw::per_pixel(p, &DevelopInputs::default(), w, h, float, points)
}
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

/// The Camera Raw shader parses and validates (naga, as wgpu does before any driver sees it),
/// with no adapter: the GPU tests below skip on runners without one.
#[test]
fn the_shader_validates_without_a_gpu() {
    let src = photocraft_gpu::CAMERA_RAW_SHADER;
    let module = wgpu::naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    assert!(!module.entry_points.is_empty());
}

#[derive(Clone, Copy, Debug)]
enum Depth {
    Eight,
    Sixteen,
    Float,
}

const DEPTHS: [Depth; 3] = [Depth::Eight, Depth::Sixteen, Depth::Float];

/// The depths the `i`th case of a set runs at: one integer depth and 32 bits. 8 and 16 bits both
/// develop with `float = false`, so they share every line of the CPU develop and the pass and
/// differ only in how [`image`] quantizes the input; cases alternate between them so both
/// quantizations still run.
fn depths(i: usize) -> [Depth; 2] {
    [DEPTHS[i % 2], Depth::Float]
}

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
fn compare(g: &mut Gpu, (p, inputs): &Case, depth: Depth) -> f32 {
    let (w, h) = (171, 113);
    let float = matches!(depth, Depth::Float);
    let src = image(w, h, depth);
    let mut cpu = src.clone();
    develop(&mut cpu, w, h, p, inputs, float);
    let points = photocraft_algo::camera_raw::white_black_points(&src, w, h, p, inputs);
    let stages = photocraft_algo::camera_raw::per_pixel(p, inputs, w, h, float, points).unwrap_or_else(|| panic!("{p:?} has a spatial stage"));
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

fn check(name: &str, sets: &[Case]) {
    let Some(mut g) = gpu() else { return };
    for (i, p) in sets.iter().enumerate() {
        for depth in depths(i) {
            let d = compare(&mut g, p, depth);
            eprintln!("{name} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
}

#[test]
fn exposure() {
    check(
        "exposure",
        &[
            case(cr! { exposure: -1.2 }, DevelopInputs::default()),
            case(cr! { exposure: 1.5 }, DevelopInputs::default()),
            case(cr! { exposure: 2.7 }, DevelopInputs { encoding_gamma: 1.8, ..Default::default() }),
            case(cr! { exposure: -3.3 }, DevelopInputs { encoding_gamma: 563.0 / 256.0, ..Default::default() }),
            case(cr! { exposure: 5.0 }, DevelopInputs::default()),
        ],
    );
}

#[test]
fn white_balance() {
    check(
        "white balance",
        &[
            case(cr! { temperature: 40.0 }, DevelopInputs::default()),
            case(cr! { tint: -25.0 }, DevelopInputs::default()),
            case(cr! { temperature: -70.0, tint: 30.0 }, DevelopInputs::default()),
            case(cr! { temperature: 100.0, tint: -100.0, exposure: 0.4 }, DevelopInputs::default()),
            case(cr! { temperature: -12.5, contrast: 30.0, whites: -40.0 }, DevelopInputs::default()),
        ],
    );
    // The measured corrections read sRGB's primaries: a document in others develops on the CPU.
    let p3 = DevelopInputs { document_primaries: Some([[0.5151, 0.2920, 0.1571], [0.2412, 0.6922, 0.0666], [-0.0011, 0.0419, 0.7843]]), ..Default::default() };
    assert!(photocraft_algo::camera_raw::per_pixel(&cr! { tint: 20.0 }, &p3, 8, 8, false, None).is_none());
}

#[test]
fn calibration() {
    check(
        "calibration",
        &[
            case(cr! { shadow_tint: 60.0 }, DevelopInputs::default()),
            case(cr! { shadow_tint: -100.0, exposure: -0.5 }, DevelopInputs::default()),
            case(cr! { calibration_hue: [40.0, -30.0, 70.0] }, DevelopInputs::default()),
            case(cr! { calibration_sat: [-100.0, 50.0, 100.0] }, DevelopInputs::default()),
            case(
                cr! { calibration_hue: [-100.0, 100.0, -50.0], calibration_sat: [30.0, -60.0, 20.0], shadow_tint: 25.0, contrast: 20.0 },
                DevelopInputs::default(),
            ),
            // Whites up measures the tones after Calibration.
            case(cr! { calibration_sat: [80.0, 80.0, 80.0], whites: 40.0, temperature: 15.0 }, DevelopInputs::default()),
        ],
    );
}

#[test]
fn contrast_without_texture_or_clarity() {
    check("contrast", &[case(cr! { contrast: 60.0 }, DevelopInputs::default()), case(cr! { contrast: -80.0, exposure: 0.4 }, DevelopInputs::default())]);
}

#[test]
fn whites_and_blacks() {
    check(
        "whites/blacks",
        &[
            case(cr! { whites: 70.0, blacks: -45.0 }, DevelopInputs::default()),
            case(cr! { whites: -60.0, blacks: 50.0, exposure: 0.2 }, DevelopInputs::default()),
            case(cr! { whites: 30.0 }, DevelopInputs::default()),
            case(cr! { blacks: -100.0, exposure: -0.5 }, DevelopInputs::default()),
            // ProPhoto's power-law encoding.
            case(cr! { whites: 50.0, blacks: -30.0 }, DevelopInputs { encoding_gamma: 1.8, ..Default::default() }),
        ],
    );
}

#[test]
fn tone_curves() {
    check(
        "tone curves",
        &[
            case(
                cr! { curve_highlights: -60.0, curve_lights: 30.0, curve_darks: 40.0, curve_shadows: -20.0, curve_splits: [20.0, 45.0, 80.0] },
                DevelopInputs::default(),
            ),
            case(cr! { point_curve: vec![[0.0, 20.0], [64.0, 50.0], [192.0, 220.0], [255.0, 245.0]] }, DevelopInputs::default()),
            case(
                cr! { point_curve_red: vec![[0.0, 0.0], [128.0, 160.0], [255.0, 255.0]], point_curve_green: vec![[0.0, 10.0], [255.0, 230.0]], point_curve_blue: vec![[0.0, 0.0], [100.0, 70.0], [255.0, 255.0]] },
                DevelopInputs::default(),
            ),
        ],
    );
}

#[test]
fn vibrance_and_saturation() {
    check(
        "presence",
        &[
            case(cr! { vibrance: 70.0 }, DevelopInputs::default()),
            case(cr! { saturation: -60.0 }, DevelopInputs::default()),
            case(cr! { vibrance: -40.0, saturation: 50.0 }, DevelopInputs::default()),
        ],
    );
}

#[test]
fn color_mixer() {
    let ramp = |k: f32| std::array::from_fn(|i| ((i as f32 * 37.0 + k) % 200.0) - 100.0);
    check(
        "color mixer",
        &[
            case(cr! { hsl_hue: ramp(0.0) }, DevelopInputs::default()),
            case(cr! { hsl_sat: ramp(50.0) }, DevelopInputs::default()),
            case(cr! { hsl_lum: ramp(120.0) }, DevelopInputs::default()),
            case(cr! { hsl_hue: ramp(10.0), hsl_sat: ramp(80.0), hsl_lum: ramp(150.0) }, DevelopInputs::default()),
        ],
    );
}

#[test]
fn color_grading() {
    check(
        "color grading",
        &[
            case(cr! { grade_shadows: Wheel { hue: 220.0, sat: 70.0, lum: 20.0 } }, DevelopInputs::default()),
            case(
                cr! { grade_midtones: Wheel { hue: 120.0, sat: 30.0, lum: -15.0 }, grade_highlights: Wheel { hue: 40.0, sat: 50.0, lum: 10.0 }, grade_global: Wheel { hue: 300.0, sat: 15.0, lum: 5.0 }, grade_blending: 80.0, grade_balance: -40.0 },
                DevelopInputs::default(),
            ),
        ],
    );
}

#[test]
fn vignette() {
    check(
        "vignette",
        &[
            case(cr! { vignette_amount: -70.0 }, DevelopInputs::default()),
            case(cr! { vignette_amount: -50.0, vignette_roundness: -60.0, vignette_feather: 20.0, vignette_highlights: 70.0 }, DevelopInputs::default()),
            case(
                cr! { vignette_amount: 60.0, vignette_roundness: 80.0, vignette_midpoint: 20.0, vignette_style: StoredVignetteStyle::ColorPriority },
                DevelopInputs::default(),
            ),
            case(cr! { vignette_amount: -40.0, vignette_style: StoredVignetteStyle::PaintOverlay }, DevelopInputs::default()),
            case(cr! { vignette_amount: 40.0, vignette_style: StoredVignetteStyle::PaintOverlay }, DevelopInputs::default()),
        ],
    );
}

#[test]
fn grain_in_full_resolution_coordinates() {
    check(
        "grain",
        &[
            case(cr! { grain_amount: 60.0 }, DevelopInputs { seed: 3, ..Default::default() }),
            // A preview proxy: the noise lattice is sampled at full-resolution positions.
            case(
                cr! { grain_amount: 80.0, grain_size: 60.0, grain_roughness: 90.0 },
                DevelopInputs { seed: u32::MAX, pixel_scale: 0.143, ..Default::default() },
            ),
            case(cr! { grain_amount: 25.0, grain_size: 0.0, grain_roughness: 0.0 }, DevelopInputs { seed: 77, pixel_scale: 0.5, ..Default::default() }),
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
        &[case(
            cr! { exposure: 0.3, contrast: 25.0, whites: 20.0, blacks: -15.0, vibrance: 30.0, saturation: 10.0, curve_darks: 20.0, point_curve: vec![[0.0, 8.0], [255.0, 250.0]], hsl_hue: hsl, hsl_sat: hsl, hsl_lum: hsl, grade_shadows: Wheel { hue: 210.0, sat: 25.0, lum: 0.0 }, vignette_amount: -30.0, grain_amount: 20.0 },
            DevelopInputs { pixel_scale: 0.2, ..Default::default() },
        )],
    );
}

#[test]
fn identity_leaves_pixels_alone_and_spatial_settings_have_no_pass() {
    let Some(mut g) = gpu() else { return };
    let p = CameraRaw::default();
    let src = image(64, 32, Depth::Float);
    let stages = per_pixel(&p, 64, 32, false, None).unwrap();
    let out = g.dev.develop(&g.device, &g.queue, None, &src, Some(1), [64, 32], &stages).unwrap();
    assert_eq!(out, src, "the CPU develop returns early, unclipped");
    for settings in [
        cr! { dehaze: 10.0 },
        cr! { shadows: 10.0 },
        cr! { highlights: -10.0 },
        cr! { texture: 10.0 },
        cr! { clarity: 10.0 },
        cr! { sharpen_amount: 10.0 },
        cr! { noise_luminance: 10.0 },
        cr! { noise_color: 10.0 },
    ] {
        assert!(spatial(&settings) && per_pixel(&settings, 64, 32, false, Some([[-8.0, -2.0, 0.0, -3.0]; 2])).is_none(), "{settings:?}");
    }
    // Contrast and Whites up need the image's points.
    assert!(per_pixel(&cr! { whites: 10.0 }, 64, 32, false, None).is_none());
    assert!(per_pixel(&cr! { contrast: 10.0 }, 64, 32, false, None).is_none());
}

#[test]
fn a_cached_source_is_reused_and_a_new_key_uploads() {
    let Some(mut g) = gpu() else { return };
    let p = cr! { exposure: 0.5 };
    let stages = per_pixel(&p, 64, 32, false, None).unwrap();
    let a = image(64, 32, Depth::Eight);
    let b: Vec<[f32; 4]> = a.iter().map(|q| [q[2], q[0], q[1], 1.0]).collect();
    let first = g.dev.develop(&g.device, &g.queue, None, &a, Some(7), [64, 32], &stages).unwrap();
    // Same key: the uploaded source stays (callers key by the proxy's identity).
    assert_eq!(g.dev.develop(&g.device, &g.queue, None, &b, Some(7), [64, 32], &stages).unwrap(), first);
    let mut cpu = b.clone();
    develop(&mut cpu, 64, 32, &p, &DevelopInputs::default(), false);
    let other = g.dev.develop(&g.device, &g.queue, None, &b, Some(8), [64, 32], &stages).unwrap();
    assert!(other.iter().zip(&cpu).all(|(x, y)| (0..3).all(|c| (x[c] - y[c]).abs() <= TOL)));
}

#[test]
fn bad_sizes_and_faulted_devices_are_errors() {
    let Some(mut g) = gpu() else { return };
    let stages = per_pixel(&cr! { exposure: 0.5 }, 4, 4, false, None).unwrap();
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

/// Develops `p` with spatial stages both ways (the GPU's spatial develop and the CPU develop) on
/// a `w × h` image and returns the largest channel difference.
fn compare_spatial(g: &mut Gpu, case: &Case, depth: Depth, (w, h): (usize, usize)) -> f32 {
    compare_spatial_image(g, case, depth, &image(w, h, depth), (w, h), None)
}

/// [`compare_spatial`] on `src` (`w × h`), uploaded under `key`.
fn compare_spatial_image(g: &mut Gpu, (p, inputs): &Case, depth: Depth, src: &[[f32; 4]], (w, h): (usize, usize), key: Option<u64>) -> f32 {
    let float = matches!(depth, Depth::Float);
    let src = src.to_vec();
    let mut cpu = src.clone();
    develop(&mut cpu, w, h, p, inputs, float);
    let out = g
        .dev
        .develop_spatial(&g.device, &g.queue, None, &src, key, [w as u32, h as u32], p, inputs, float)
        .unwrap()
        .unwrap_or_else(|| panic!("{p:?}: no spatial develop"));
    assert_eq!(out.len(), cpu.len());
    let mut worst = (0.0f32, 0usize);
    let mut over = 0usize;
    for (i, (a, b)) in out.iter().zip(&cpu).enumerate() {
        assert!(a.iter().all(|v| v.is_finite()), "{depth:?}: non-finite GPU pixel {i}: {a:?}");
        assert_eq!(a[3], b[3], "alpha is kept");
        let d = (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0f32, f32::max);
        if d > SPATIAL_TOL {
            over += 1;
        }
        if d > worst.0 {
            worst = (d, i);
        }
    }
    let (d, i) = worst;
    eprintln!("  {over} of {} pixels over {SPATIAL_TOL:.1e}; worst {d:.2e} at {i}: GPU {:?} CPU {:?} source {:?}", out.len(), out[i], cpu[i], src[i]);
    assert!(
        over * 10_000 <= out.len() && d <= 1.0 / 255.0,
        "{p:?} {depth:?} {w}×{h}: GPU {:?} vs CPU {:?} at pixel {i}, off by {d}; {over} pixels over",
        out[i],
        cpu[i]
    );
    d
}

/// The spatial stages' analyses read reduced copies the GPU made, which differ from the CPU's
/// in the last bits (f32 sums, the GPU's `pow`); the analyses (sorts, guided filters) carry that
/// into the result. A pixel near grey can turn that into a hue change the Color Mixer and
/// Vibrance tables then weigh differently, so a few pixels in ten thousand may go past this,
/// none by an 8-bit step.
const SPATIAL_TOL: f32 = 1.0 / 1024.0;

fn check_spatial(name: &str, sets: &[Case]) {
    let Some(mut g) = gpu() else { return };
    // Small enough that every reduction is one pixel per pixel, and large enough that each
    // reduces (Dehaze's copy at most 768, the tones' 384).
    // The long side decides the reductions (Dehaze's copy past 768, the tones' past 384), so the
    // large frame is a strip.
    for size in [(171, 113), (1700, 300)] {
        for (i, set) in sets.iter().enumerate() {
            for depth in depths(i) {
                let d = compare_spatial(&mut g, set, depth, size);
                eprintln!("{name} {depth:?} {}×{}: max |GPU - CPU| = {d:.2e}", size.0, size.1);
            }
        }
    }
}

#[test]
fn dehaze_on_the_gpu() {
    check_spatial(
        "dehaze",
        &[
            case(cr! { dehaze: 40.0 }, DevelopInputs::default()),
            case(cr! { dehaze: 100.0, exposure: 0.3 }, DevelopInputs::default()),
            case(cr! { dehaze: -30.0 }, DevelopInputs::default()),
            case(cr! { dehaze: -100.0, temperature: 20.0 }, DevelopInputs::default()),
        ],
    );
}

#[test]
fn highlights_and_shadows_on_the_gpu() {
    check_spatial(
        "tone",
        &[
            case(cr! { highlights: -60.0 }, DevelopInputs::default()),
            case(cr! { shadows: 70.0 }, DevelopInputs::default()),
            case(cr! { highlights: -100.0, shadows: 100.0 }, DevelopInputs::default()),
            case(cr! { highlights: 50.0, shadows: -40.0, contrast: 30.0 }, DevelopInputs::default()),
            case(cr! { shadows: 50.0 }, DevelopInputs { encoding_gamma: 1.8, ..Default::default() }),
        ],
    );
}

#[test]
fn spatial_stages_with_everything_after() {
    let mut hsl = [0.0; 8];
    hsl[1] = 35.0;
    check_spatial(
        "spatial and after",
        &[
            case(
                cr! { dehaze: 30.0, highlights: -40.0, shadows: 40.0, whites: 30.0, blacks: -20.0, vibrance: 25.0, hsl_sat: hsl, vignette_amount: -30.0 },
                DevelopInputs::default(),
            ),
            case(
                cr! { shadows: 60.0, whites: -50.0, calibration_sat: [20.0, 0.0, -20.0], grain_amount: 20.0 },
                DevelopInputs { seed: 9, ..Default::default() },
            ),
        ],
    );
}

#[test]
fn texture_and_clarity_on_the_gpu() {
    check_spatial(
        "detail",
        &[
            case(cr! { texture: 60.0 }, DevelopInputs::default()),
            case(cr! { texture: -100.0 }, DevelopInputs::default()),
            case(cr! { clarity: 50.0 }, DevelopInputs::default()),
            case(cr! { clarity: -80.0, exposure: 0.3 }, DevelopInputs::default()),
            case(cr! { texture: 40.0, clarity: 40.0, shadows: 30.0, dehaze: 20.0, whites: 25.0 }, DevelopInputs::default()),
        ],
    );
}

/// The spatial stages on frames down to one pixel, odd sizes and a flat frame, as the CPU
/// develops them (a frame under 2 pixels on a side skips Shadows and Highlights there too).
#[test]
fn spatial_stages_on_tiny_odd_and_flat_frames() {
    let Some(mut g) = gpu() else { return };
    let sets = [
        case(cr! { shadows: 60.0, highlights: -40.0, whites: 20.0 }, DevelopInputs::default()),
        case(cr! { dehaze: 45.0, clarity: 50.0, texture: 60.0, whites: 20.0, blacks: -20.0 }, DevelopInputs::default()),
        case(cr! { dehaze: -45.0, shadows: 30.0 }, DevelopInputs::default()),
    ];
    for set in &sets {
        // One depth per size, in turn: the size decides the reductions and the edges, the depth
        // only the input's quantization, which the other tests run at every depth.
        for (i, size) in [(1, 1), (2, 2), (40, 2), (3, 3), (5, 3), (37, 23), (300, 7)].into_iter().enumerate() {
            let depth = DEPTHS[i % DEPTHS.len()];
            let d = compare_spatial(&mut g, set, depth, size);
            eprintln!("tiny/odd {size:?} {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
        // A flat frame: no range to filter, no haze to find, no detail to raise. It isn't
        // quantized, so one integer depth.
        let flat = vec![[0.4, 0.35, 0.3, 1.0]; 64 * 48];
        for depth in depths(0) {
            compare_spatial_image(&mut g, set, depth, &flat, (64, 48), None);
        }
    }
    // Texture's blur on a small frame can be too narrow to change anything; the stage still runs.
    for depth in depths(1) {
        compare_spatial(&mut g, &case(cr! { texture: 80.0, contrast: 10.0 }, DevelopInputs::default()), depth, (64, 48));
    }
}

/// #9273: a sky beside a dark tower clears on the GPU as on the CPU (the dark channel is opened
/// the same way, since the CPU analyses the GPU's reduced copy). The scene of algo's
/// `dehaze_leaves_no_halo_beside_dark_objects`: a clear sky and a dark tower hazed with
/// `I = J·t + A·(1 − t)`.
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
    for (i, dehaze) in [100.0, 60.0, -40.0].into_iter().enumerate() {
        for depth in depths(i) {
            let d = compare_spatial_image(&mut g, &case(cr! { dehaze: dehaze }, DevelopInputs::default()), depth, &src, (w, h), None);
            eprintln!("dehaze {dehaze} on the tower {depth:?}: max |GPU - CPU| = {d:.2e}");
        }
    }
}

/// The cached source (same key) is developed again when the white balance changes: the reduced
/// copies are made from the pass's result each time, not kept with the source.
#[test]
fn dehaze_follows_the_white_balance_on_a_cached_source() {
    let Some(mut g) = gpu() else { return };
    let src = image(858, 572, Depth::Eight);
    for temperature in [25.0, -40.0, -40.0, 0.0] {
        compare_spatial_image(&mut g, &case(cr! { dehaze: 50.0, temperature: temperature }, DevelopInputs::default()), Depth::Eight, &src, (858, 572), Some(3));
    }
}

/// `cargo test -p photocraft-gpu --release --test it -- --ignored --nocapture spatial_timing`
#[test]
#[ignore = "timing"]
fn spatial_timing() {
    let Some(mut g) = gpu() else { return };
    let (w, h) = (2000, 1333);
    let src = image(w, h, Depth::Eight);
    let all = cr! { exposure: 0.3, contrast: 20.0, dehaze: 20.0, highlights: -40.0, shadows: 40.0, texture: 30.0, clarity: 30.0, whites: 20.0, blacks: -10.0, vibrance: 20.0 };
    for (name, p) in [
        ("Shadows/Highlights", cr! { highlights: -40.0, shadows: 40.0 }),
        ("Dehaze", cr! { dehaze: 30.0 }),
        ("Texture", cr! { texture: 40.0 }),
        ("Clarity", cr! { clarity: 40.0 }),
        ("all of them", all),
    ] {
        let inputs = DevelopInputs::default();
        let time = |f: &mut dyn FnMut()| {
            f();
            let t = std::time::Instant::now();
            for _ in 0..5 {
                f();
            }
            t.elapsed().as_secs_f64() * 1000.0 / 5.0
        };
        let gpu_ms = time(&mut || {
            std::hint::black_box(g.dev.develop_spatial(&g.device, &g.queue, None, &src, Some(1), [w as u32, h as u32], &p, &inputs, false).unwrap());
        });
        let cpu_ms = time(&mut || {
            let mut px = src.clone();
            develop(&mut px, w, h, &p, &inputs, false);
        });
        eprintln!("{name:20} {w}×{h}: GPU {gpu_ms:7.1} ms, CPU {cpu_ms:7.1} ms");
    }
    // The per-pixel pass alone (upload cached, one pass, the read back), for the fixed cost.
    let p = cr! { exposure: 0.3 };
    let stages = per_pixel(&p, w, h, false, None).unwrap();
    let t = std::time::Instant::now();
    for _ in 0..5 {
        std::hint::black_box(g.dev.develop(&g.device, &g.queue, None, &src, Some(1), [w as u32, h as u32], &stages).unwrap());
    }
    eprintln!("per-pixel pass alone: {:7.1} ms", t.elapsed().as_secs_f64() * 1000.0 / 5.0);
}
