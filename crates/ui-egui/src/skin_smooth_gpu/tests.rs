use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_plugins::{Limits, Plugin, acceleration};
use photocraft_raster::Surface;
use serde_json::json;

const WASM: &[u8] = include_bytes!("../../../plugins/tests/fixtures/skin-smooth.wasm");

#[test]
fn sampled_colour_ranges_match_gpu_and_wasm_including_subtract_only() {
    let _gpu_lock = photocraft_testkit::gpu_lock();
    let backend = backend();
    let plugin = Plugin::load(WASM, Limits::default()).unwrap();
    let rect = Rect::new(0, 0, 48, 32);
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut source = Surface::new(PixelFormat { mode: ColorMode::Rgb, sample, alpha: true });
        let pixels: Vec<f32> = (0..48 * 32)
            .flat_map(|i| {
                let t = (i % 48) as f32 / 47.0;
                [0.15 + 0.6 * t, 0.35 + 0.3 * t, 0.8 - 0.5 * t, (i % 7) as f32 / 6.0]
            })
            .collect();
        source.write_region(rect, &pixels);
        for (positive, negative) in [
            (json!([]), json!([])),
            (json!([[0.2, 0.4, 0.8]]), json!([])),
            (json!([[0.2, 0.4, 0.8], [0.7, 0.5, 0.4]]), json!([[0.55, 0.55, 0.55]])),
            (json!([]), json!([[0.7, 0.5, 0.4]])),
            (json!([[0.2, 0.4, 0.8], [0.2, 0.4, 0.8]]), json!([[0.2, 0.4, 0.8]])),
        ] {
            for tolerance in [0, 55, 100] {
                let params = json!({"positiveColors":positive,"negativeColors":negative,"tolerance":tolerance,"brightness":35,"warmth":12});
                let expected = plugin.apply(&source, rect, None, &params).unwrap();
                let result = {
                    let _scope = acceleration::scope(Some(backend.clone()));
                    plugin.apply(&source, rect, None, &params).unwrap()
                };
                let max = expected.read_region(rect).iter().zip(result.read_region(rect)).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
                let epsilon = match sample {
                    SampleType::U8 => 1.0 / 255.0 + 3e-6,
                    SampleType::U16 => 1.0 / 65535.0 + 3e-6,
                    SampleType::F32 => 3e-6,
                };
                assert!(max <= epsilon, "{sample:?} tolerance {tolerance}: {max}");
            }
        }
    }
    assert!(backend.reported.load(Ordering::Relaxed));
    assert!(!backend.failed.load(Ordering::Relaxed));
}

fn backend() -> Arc<Backend> {
    let rs = egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default());
    eprintln!("Skin Smooth test adapter: {}", rs.adapter.get_info().name);
    Arc::new(Backend {
        device: rs.device.clone(),
        queue: rs.queue.clone(),
        health: DeviceHealth::watch(&rs.device),
        pass: Mutex::new(None),
        failed: AtomicBool::new(false),
        reported: AtomicBool::new(false),
    })
}

#[test]
fn gpu_bands_match_wasm_for_depths_offsets_seams_selection_and_alpha() {
    let _gpu_lock = photocraft_testkit::gpu_lock();
    let backend = backend();
    let plugin = Plugin::load(WASM, Limits::default()).unwrap();
    let params = json!({"fine":100,"medium":100,"large":100,"detail":0,"skinOnly":false,"warmth":8,"brightness":15,"redness":20});
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (x, y, selected, alpha) in [(0, 0, false, true), (-128, -64, true, true), (128, 64, false, true), (0, 0, false, false)] {
            let rect = Rect::new(x, y, x + 1050, y + 280);
            let canvas = Rect::new(0, 0, 1050, 280);
            let fmt = PixelFormat { mode: ColorMode::Rgb, sample, alpha };
            let mut input = Surface::new(fmt);
            let values: Vec<f32> = (0..1050 * 280)
                .flat_map(|i| {
                    let (x, y) = (i % 1050, i / 1050);
                    let noise = ((x * 17 + y * 13) % 9) as f32 - 4.;
                    [(175. + noise) / 255., (120. + noise) / 255., (92. + noise) / 255., if i % 11 == 0 { 0. } else { (x % 7) as f32 / 6. }]
                })
                .collect();
            let values = if alpha { values } else { values.chunks_exact(4).flat_map(|p| p[..3].iter().copied()).collect() };
            input.write_region(rect, &values);
            let mut mask = Surface::new(PixelFormat::GRAY8);
            mask.fill_rect(Rect::new(37, 43, 989, 251), &[0.4]);
            let selection = selected.then_some(&mask);
            let expected = plugin.apply(&input, canvas, selection, &params).unwrap();
            let result = {
                let _scope = acceleration::scope(Some(backend.clone()));
                plugin.apply(&input, canvas, selection, &params).unwrap()
            };
            let (a, b) = (expected.read_region(rect), result.read_region(rect));
            let tolerance = match sample {
                SampleType::U8 => 1. / 255. + 2e-6,
                SampleType::U16 => 1. / 65535. + 2e-6,
                SampleType::F32 => 2e-6,
            };
            let mut max = 0f32;
            for (i, (a, b)) in a.iter().zip(&b).enumerate() {
                if alpha && i % 4 == 3 {
                    assert_eq!(a.to_bits(), b.to_bits(), "alpha {sample:?}");
                } else {
                    max = max.max((a - b).abs());
                }
            }
            assert!(max <= tolerance, "{sample:?} offset {x},{y}, delta {max}");
        }
    }
    assert!(backend.reported.load(Ordering::Relaxed), "GPU must actually execute");
    assert!(!backend.failed.load(Ordering::Relaxed));
}

#[test]
fn failed_device_falls_back_and_cancel_commits_no_partial_output() {
    let _gpu_lock = photocraft_testkit::gpu_lock();
    let backend = backend();
    let plugin = Plugin::load(WASM, Limits::default()).unwrap();
    let rect = Rect::new(0, 0, 64, 48);
    let mut input = Surface::new(PixelFormat::RGBA8);
    input.fill_rect(rect, &[0.7, 0.48, 0.36, 0.6]);
    let expected = plugin.apply(&input, rect, None, &json!({})).unwrap();
    backend.health.mark(photocraft_gpu::Fault::Lost("test".into()));
    let _scope = acceleration::scope(Some(backend));
    assert_eq!(plugin.apply(&input, rect, None, &json!({})).unwrap().read_region(rect), expected.read_region(rect));
    assert!(matches!(plugin.apply_with(&input, rect, None, &json!({}), &Interrupt::cancel_only(&|| true)), Err(Error::Cancelled)));
}

#[test]
fn pending_readback_cancellation_retires_the_pass_before_fallback() {
    use std::sync::atomic::AtomicUsize;
    let _gpu_lock = photocraft_testkit::gpu_lock();
    let backend = backend();
    let plugin = Plugin::load(WASM, Limits::default()).unwrap();
    let params = Value::Object(plugin.manifest().resolve_params(&json!({"preset":"manual","brightness":15})).unwrap());
    let mut pixels = [0.7, 0.48, 0.36, 1.0].repeat(16);
    let before = pixels.clone();
    let checks = AtomicUsize::new(0);
    // Entry, lock acquisition and pre-submit succeed; cancel only in the readback wait.
    let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 3;
    let band = Band { pixels: &mut pixels, size: [4, 4], channels: 4, format: 8 | 256 | (3 << 16), params: &params };
    assert!(matches!(backend.filter(band, &Interrupt::cancel_only(&cancel)), Err(Error::Cancelled)));
    assert!(checks.load(Ordering::Relaxed) > 3);
    assert_eq!(pixels, before);
    assert!(backend.failed.load(Ordering::Relaxed));
    assert!(backend.pass.lock().unwrap().is_none());
    let band = Band { pixels: &mut pixels, size: [4, 4], channels: 4, format: 8 | 256 | (3 << 16), params: &params };
    assert!(!backend.filter(band, &Interrupt::NONE).unwrap(), "later requests must use WASM");
}

#[test]
fn cancellation_does_not_wait_for_another_gpu_invocations_mutex() {
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;
    let _gpu_lock = photocraft_testkit::gpu_lock();
    let backend = backend();
    let plugin = Plugin::load(WASM, Limits::default()).unwrap();
    let params = Value::Object(plugin.manifest().resolve_params(&json!({"preset":"manual","brightness":15})).unwrap());
    let mut pixels = [0.7, 0.48, 0.36, 1.0].repeat(16);
    let checks = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let (waiting_tx, waiting_rx) = std::sync::mpsc::sync_channel(1);
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
    let cancel = || {
        if checks.fetch_add(1, Ordering::Relaxed) == 2 {
            let _ = waiting_tx.send(());
        }
        cancelled.load(Ordering::Relaxed)
    };
    let held = backend.pass.lock().unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let band = Band { pixels: &mut pixels, size: [4, 4], channels: 4, format: 8 | 256 | (3 << 16), params: &params };
            let _ = done_tx.send(backend.filter(band, &Interrupt::cancel_only(&cancel)));
        });
        let waiting = waiting_rx.recv_timeout(Duration::from_secs(2));
        cancelled.store(true, Ordering::Relaxed);
        let stopped = done_rx.recv_timeout(Duration::from_secs(2));
        drop(held); // Release even on failure so a regression cannot strand the test thread.
        assert!(waiting.is_ok(), "the request must reach the contended mutex");
        assert!(matches!(stopped, Ok(Err(Error::Cancelled))), "cancel must finish while the lock is held");
    });
    assert!(!backend.failed.load(Ordering::Relaxed), "a cancelled waiter must not retire another request's pass");
}

#[test]
fn session_jobs_preview_and_cpu_preference_route_to_the_correct_backend() {
    let _gpu_lock = photocraft_testkit::gpu_lock();
    use photocraft_engine::{Session, jobs::Started, prefs::RenderingMode};
    photocraft_plugins::registry::install_bytes(WASM).unwrap();
    let backend = backend();
    let mut s = Session::new();
    s.plugin_accelerator = Some(backend.clone());
    s.execute("file.new", json!({"width":64,"height":48,"background":"#b17d62"})).unwrap();
    s.execute("edit.assignProfile", json!({"profile":"srgb"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let layer = s.active().unwrap().active_layer;
    let params = json!({"id":"local.skinsmooth.skin-smooth","brightness":20});
    let Started::Job(job) = s.start("plugin.run", params.clone()).unwrap() else { panic!("expected background job") };
    s.wait_job(job).unwrap();
    assert!(backend.reported.load(Ordering::Relaxed));
    let px = |s: &Session| s.active().unwrap().doc.layer(layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1);
    let original = before.layer(layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1);
    assert_ne!(px(&s), original);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(px(&s), original);
    let preview = {
        let _scope = acceleration::scope(Some(backend.clone()));
        crate::filter_dialog::preview_document(&before, layer, "plugin.run", &params, 1).unwrap()
    };
    assert_ne!(preview.layer(layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1), original);
    assert_eq!(px(&s), original);
    backend.reported.store(false, Ordering::Relaxed);
    s.edit_prefs(|p| p.performance.rendering_mode = Some(RenderingMode::Cpu));
    assert!(s.effective_plugin_accelerator().is_none());
    let Started::Job(job) = s.start("plugin.run", params).unwrap() else { panic!("expected CPU job") };
    s.wait_job(job).unwrap();
    assert!(!backend.reported.load(Ordering::Relaxed));
    assert_ne!(px(&s), original);
}

#[test]
#[ignore = "GPU performance: run alone on a hardware adapter with --nocapture"]
fn skin_smooth_gpu_benchmark() {
    let _gpu_lock = photocraft_testkit::gpu_lock();
    let backend = backend();
    let plugin = Plugin::load(WASM, Limits::default()).unwrap();
    let raw = std::env::var_os("SKIN_SMOOTH_BENCH_RGBA").map(|p| std::fs::read(p).unwrap());
    if let Some(raw) = &raw {
        assert_eq!(raw.len(), 5760 * 3840 * 4);
    }
    for (w, h) in [(1920, 1080), (3840, 2160), (5760, 3840)] {
        let rect = Rect::new(0, 0, w, h);
        let mut input = Surface::new(PixelFormat::RGBA8);
        let pixels: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                if let Some(raw) = &raw {
                    let j = ((y as usize * 3840 / h as usize) * 5760 + x as usize * 5760 / w as usize) * 4;
                    [raw[j] as f32 / 255., raw[j + 1] as f32 / 255., raw[j + 2] as f32 / 255., raw[j + 3] as f32 / 255.]
                } else {
                    let n = ((x * 17 + y * 13) % 9) as f32 - 4.;
                    [(175. + n) / 255., (120. + n) / 255., (92. + n) / 255., 1.]
                }
            })
            .collect();
        input.write_region(rect, &pixels);
        drop(pixels);
        let mut mask = Surface::new(PixelFormat::GRAY8);
        mask.fill_rect(Rect::new((w - 800) / 2, (h - 600) / 2, (w + 800) / 2, (h + 600) / 2), &[1.]);
        for (mode, selection) in [("full", None), ("viewport", Some(&mask))] {
            let t = std::time::Instant::now();
            let expected = plugin.apply(&input, rect, selection, &json!({})).unwrap();
            let cpu = t.elapsed().as_secs_f64();
            let mut times = Vec::new();
            let mut max = 0f32;
            for _ in 0..3 {
                let _scope = acceleration::scope(Some(backend.clone()));
                let t = std::time::Instant::now();
                let out = plugin.apply(&input, rect, selection, &json!({})).unwrap();
                times.push(t.elapsed().as_secs_f64());
                for y in (0..h).step_by(256) {
                    let row = Rect::new(0, y, w, (y + 256).min(h));
                    let (a, b) = (expected.read_region(row), out.read_region(row));
                    for (i, (a, b)) in a.iter().zip(&b).enumerate() {
                        if i % 4 == 3 {
                            assert_eq!(a.to_bits(), b.to_bits());
                        } else {
                            max = max.max((a - b).abs());
                        }
                    }
                }
            }
            assert!(backend.reported.load(Ordering::Relaxed));
            assert!(!backend.failed.load(Ordering::Relaxed));
            assert!(max <= 1. / 255. + 2e-6);
            times.sort_by(f64::total_cmp);
            println!(
                "{}",
                json!({"size":[w,h],"mode":mode,"wasmSeconds":cpu,"gpuSeconds":times,"gpuMedianSeconds":times[1],"maxRgb":max,"realPhoto":raw.is_some()})
            );
        }
    }
}

#[test]
fn built_in_presets_override_sliders_on_gpu_and_wasm() {
    let _gpu_lock = photocraft_testkit::gpu_lock();
    let backend = backend();
    let plugin = Plugin::load(WASM, Limits::default()).unwrap();
    let rect = Rect::new(0, 0, 48, 32);
    let mut source = Surface::new(PixelFormat { sample: SampleType::F32, ..PixelFormat::RGBA8 });
    let pixels: Vec<f32> = (0..48 * 32)
        .flat_map(|i| {
            let n = (i % 9) as f32 / 100.0;
            [0.65 + n, 0.42 + n, 0.3 + n, (i % 7) as f32 / 6.0]
        })
        .collect();
    source.write_region(rect, &pixels);
    for (preset, values) in [
        ("natural", json!({"fine":45,"medium":25,"large":10,"detail":20,"brightness":0,"warmth":0,"redness":0,"tolerance":55,"skinOnly":true})),
        ("soft", json!({"fine":70,"medium":50,"large":20,"detail":10,"brightness":0,"warmth":8,"redness":0,"tolerance":55,"skinOnly":true})),
        ("clean", json!({"fine":40,"medium":20,"large":10,"detail":30,"brightness":10,"warmth":0,"redness":20,"tolerance":55,"skinOnly":true})),
    ] {
        let expected = plugin.apply(&source, rect, None, &values).unwrap().read_region(rect);
        assert_ne!(expected, source.read_region(rect), "preset {preset} must change the synthetic image");
        let params =
            json!({"preset":preset,"fine":0,"medium":0,"large":0,"detail":0,"brightness":-100,"warmth":-100,"redness":100,"tolerance":0,"skinOnly":false});
        let wasm = plugin.apply(&source, rect, None, &params).unwrap().read_region(rect);
        assert_eq!(wasm, expected, "WASM preset {preset}");
        let gpu = {
            let _scope = acceleration::scope(Some(backend.clone()));
            plugin.apply(&source, rect, None, &params).unwrap().read_region(rect)
        };
        for (i, (&actual, &expected)) in gpu.iter().zip(&expected).enumerate() {
            if i % 4 == 3 {
                assert_eq!(actual.to_bits(), expected.to_bits(), "alpha for {preset}");
            } else {
                assert!((actual - expected).abs() <= 3e-6, "{preset} channel {i}: {actual} vs {expected}");
            }
        }
    }
    assert!(backend.reported.load(Ordering::Relaxed));
    assert!(!backend.failed.load(Ordering::Relaxed));
}
