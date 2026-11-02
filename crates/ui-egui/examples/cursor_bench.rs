//! Release benchmark harness measuring cold rasterization, warm movement lookup, and painted fallback
//! baseline for native Pen tool cursors (#9122).
//!
//! Evaluates all six standard Pen tool cursor variants across multiple display scales, measuring:
//! 1. Cold rasterization latency and bitmap dimensions/hotspots.
//! 2. Warm-cache lookup latency during steady pointer movements across frames.
//! 3. Painted fallback baseline latency using identical pointer moves and harness.
//! 4. Structural gate: zero additional rasterizations during steady warm movement with unchanged appearance.
//! 5. Cost gate: warm-path p95 below 0.1 ms (100 µs).
//! 6. Optional artifact export of cursor images as PNG files for visual regression inspection.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example cursor_bench -- [--moves 1000] [--reps 5] [--save-artifacts <dir>] [--json [path]]
//! ```

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use egui::{Context, pos2};
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType, encode};
use photocraft_testkit::gpu_lock;
use photocraft_ui_egui::canvas::tool_cursor::{PenVariant, native_scale, pen, pen_rasterization_count, rasterize_pen, reset_pen_rasterization_count};
use photocraft_ui_egui::icons;
use photocraft_ui_egui::state::Tool;

#[derive(Clone, Debug, serde::Serialize)]
struct BenchmarkRun {
    variant: String,
    tool: String,
    scale: f32,
    measured_native_scale: f32,
    dimensions: [u16; 2],
    hotspot: [u16; 2],
    retained_bytes: usize,
    cold_raster_us_samples: Vec<f64>,
    cold_raster_p50_us: f64,
    cold_raster_p95_us: f64,
    warm_move_us_samples: Vec<f64>,
    warm_move_p50_us: f64,
    warm_move_p95_us: f64,
    baseline_painted_us_samples: Vec<f64>,
    baseline_painted_p50_us: f64,
    baseline_painted_p95_us: f64,
    preparation_speedup_vs_baseline_p50: f64,
    warm_moves_total: usize,
    cache_hits: usize,
    warm_rasterizations: usize,
    structural_gate_pass: bool,
    cost_gate_pass: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
struct PerfReportRow {
    name: String,
    samples_ms: Vec<f64>,
    p50_ms: f64,
    p95_ms: f64,
}

#[derive(Clone, Debug, serde::Serialize)]
struct CursorVisualComparison {
    variant: String,
    scale: f32,
    background: String,
    dimensions: [u32; 2],
    hotspot: [u32; 2],
    mae_rgb: f64,
    max_abs_diff_rgb: u8,
    rmse_rgb: f64,
    exact_match_pixels: usize,
    total_pixels: usize,
    match_percentage: f64,
    pixels_diff_gt_2: usize,
    pixels_diff_gt_5: usize,
    pixels_diff_gt_15: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
struct PerfReport {
    context: serde_json::Value,
    rows: Vec<PerfReportRow>,
    results: Vec<BenchmarkRun>,
    visual_comparisons: Vec<CursorVisualComparison>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contact_sheet_png_base64: Option<String>,
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * pct).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn tool_for_variant(variant: PenVariant) -> Tool {
    match variant {
        PenVariant::Standard => Tool::Pen,
        PenVariant::Freeform => Tool::FreeformPen,
        PenVariant::Curvature => Tool::CurvaturePen,
        PenVariant::AddAnchor => Tool::AddAnchorPoint,
        PenVariant::DeleteAnchor => Tool::DeleteAnchorPoint,
        PenVariant::ConvertPoint => Tool::ConvertPoint,
    }
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

fn save_cursor_artifact(output_dir: &Path, variant_name: &str, scale: f32, image: &egui::CustomCursorImage) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(output_dir)?;
    let [w, h] = image.size;
    let raw_bytes = image.rgba.to_vec();
    let img = Image::from_raw(u32::from(w), u32::from(h), ChannelLayout::Rgba, SampleType::U8, raw_bytes)?;
    let encoded = encode(&img, Format::Png, &EncodeOptions::default())?;
    let filename = format!("cursor_pen_{variant_name}_{scale:.2}x.png");
    let file_path = output_dir.join(filename);
    std::fs::write(&file_path, encoded)?;
    Ok(())
}

// Execute one event-loop frame. Timings cover cursor preparation only, not GPU or
// presentation latency. Each frame drains output so painted geometry cannot accumulate.
fn cursor_frame(ctx: &Context, tool: Tool, variant: PenVariant, at: egui::Pos2, painted: bool) -> (f64, Option<egui::CustomCursorImage>) {
    let mut elapsed_us = 0.0;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(512.0, 512.0))),
        events: vec![egui::Event::PointerMoved(at)],
        ..Default::default()
    };
    let output = ctx.run(input, |ctx| {
        ctx.set_cursor_image(None);
        let start = Instant::now();
        if painted {
            icons::cursor(ctx, variant.icon_name(), at, variant.hotspot_in_box() / 24.0, 22.0);
        } else if let Some(icon) = pen(ctx, tool) {
            ctx.set_cursor_icon(icon);
        }
        elapsed_us = start.elapsed().as_secs_f64() * 1_000_000.0;
    });
    (elapsed_us, output.platform_output.cursor_image)
}

fn run_visual_oracle(save_dir: Option<&Path>) -> (Vec<CursorVisualComparison>, Option<String>) {
    let _gpu = gpu_lock();

    let scales = [1.0f32, 1.25, 1.5, 2.0];
    let variants = PenVariant::all();
    let backgrounds = [
        ("light_240", egui::Color32::from_rgb(240, 240, 240)),
        ("midgrey_128", egui::Color32::from_rgb(128, 128, 128)),
        ("dark_15", egui::Color32::from_rgb(15, 15, 15)),
    ];

    let mut comparisons = Vec::new();
    let mut rendered_scale_images: Vec<(f32, Vec<u8>, u32, u32)> = Vec::new();

    for &scale in &scales {
        let canvas_w = 640.0;
        let canvas_h = 320.0;

        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(canvas_w, canvas_h)).with_pixels_per_point(scale).wgpu().build_ui(|ui| {
                ui.painter().rect_filled(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(canvas_w, canvas_h)), 0.0, egui::Color32::from_rgb(40, 40, 40));

                for (bg_idx, (_bg_name, bg_color)) in backgrounds.iter().enumerate() {
                    let y_top = 25.0 + (bg_idx as f32) * 95.0;
                    let bg_rect = egui::Rect::from_min_size(egui::pos2(10.0, y_top), egui::vec2(canvas_w - 20.0, 90.0));
                    ui.painter().rect_filled(bg_rect, 0.0, *bg_color);

                    for (var_idx, variant) in variants.iter().enumerate() {
                        let cell_x = 20.0 + (var_idx as f32) * 100.0;
                        let y_center = y_top + 45.0;

                        // Left: Painted baseline
                        let p_painted = egui::pos2(cell_x + 24.0, y_center);
                        icons::cursor(ui.ctx(), variant.icon_name(), p_painted, variant.hotspot_in_box() / 24.0, 22.0);

                        // Right: Native cursor bitmap displayed at 1:1 pixel scale
                        if let Some(native_img) = rasterize_pen(*variant, scale) {
                            let [w, h] = native_img.size;
                            let [hx, hy] = native_img.hotspot;
                            let color_img = egui::ColorImage::from_rgba_unmultiplied([usize::from(w), usize::from(h)], &native_img.rgba);
                            let tex = ui.ctx().load_texture(
                                format!("bench_native_{}_{scale}_{bg_idx}", variant.icon_name()),
                                color_img,
                                egui::TextureOptions::NEAREST,
                            );
                            let p_native = egui::pos2(cell_x + 68.0, y_center);
                            let native_rect = egui::Rect::from_min_size(
                                p_native - egui::vec2(hx as f32 / scale, hy as f32 / scale),
                                egui::vec2(w as f32 / scale, h as f32 / scale),
                            );
                            ui.painter().image(tex.id(), native_rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                        }
                    }
                }
            });
            h.run_steps(3);
            h.render()
        }));

        let Ok(Ok(rendered_img)) = built else {
            eprintln!("wgpu rendering unavailable on this platform; skipping visual reference generation");
            return (Vec::new(), None);
        };

        let img_w = rendered_img.width();
        let img_h = rendered_img.height();
        let raw_pixels = rendered_img.into_raw();

        // Compute compare metrics for each variant and background
        for (bg_idx, (bg_name, _)) in backgrounds.iter().enumerate() {
            let y_top = 25.0 + (bg_idx as f32) * 95.0;
            let y_center = y_top + 45.0;

            for (var_idx, variant) in variants.iter().enumerate() {
                let cell_x = 20.0 + (var_idx as f32) * 100.0;
                let p_painted = egui::pos2(cell_x + 24.0, y_center);
                let p_native = egui::pos2(cell_x + 68.0, y_center);

                let Some(native_img) = rasterize_pen(*variant, scale) else {
                    continue;
                };
                let [w, h] = native_img.size;
                let [hx, hy] = native_img.hotspot;

                let px_hot_painted = (p_painted.x * scale).round() as i32;
                let py_hot_painted = (p_painted.y * scale).round() as i32;
                let px_hot_native = (p_native.x * scale).round() as i32;
                let py_hot_native = (p_native.y * scale).round() as i32;

                let mut sum_abs_diff = 0.0f64;
                let mut sum_sq_diff = 0.0f64;
                let mut max_diff: u8 = 0;
                let mut exact_matches = 0;
                let mut diff_gt_2 = 0;
                let mut diff_gt_5 = 0;
                let mut diff_gt_15 = 0;
                let total_pixels = usize::from(w) * usize::from(h);

                for dy in 0..(h as i32) {
                    let rel_y = dy - (hy as i32);
                    let y_p = py_hot_painted + rel_y;
                    let y_n = py_hot_native + rel_y;

                    for dx in 0..(w as i32) {
                        let rel_x = dx - (hx as i32);
                        let x_p = px_hot_painted + rel_x;
                        let x_n = px_hot_native + rel_x;

                        if x_p >= 0 && x_p < img_w as i32 && y_p >= 0 && y_p < img_h as i32 && x_n >= 0 && x_n < img_w as i32 && y_n >= 0 && y_n < img_h as i32
                        {
                            let idx_p = ((y_p as u32 * img_w + x_p as u32) * 4) as usize;
                            let idx_n = ((y_n as u32 * img_w + x_n as u32) * 4) as usize;

                            let dr = (raw_pixels[idx_p] as i16 - raw_pixels[idx_n] as i16).unsigned_abs() as u8;
                            let dg = (raw_pixels[idx_p + 1] as i16 - raw_pixels[idx_n + 1] as i16).unsigned_abs() as u8;
                            let db = (raw_pixels[idx_p + 2] as i16 - raw_pixels[idx_n + 2] as i16).unsigned_abs() as u8;

                            let pixel_max = dr.max(dg).max(db);
                            let avg_diff = (dr as f64 + dg as f64 + db as f64) / 3.0;
                            sum_abs_diff += avg_diff;
                            sum_sq_diff += ((dr as f64).powi(2) + (dg as f64).powi(2) + (db as f64).powi(2)) / 3.0;

                            if pixel_max > max_diff {
                                max_diff = pixel_max;
                            }
                            if pixel_max == 0 {
                                exact_matches += 1;
                            }
                            if pixel_max > 2 {
                                diff_gt_2 += 1;
                            }
                            if pixel_max > 5 {
                                diff_gt_5 += 1;
                            }
                            if pixel_max > 15 {
                                diff_gt_15 += 1;
                            }
                        }
                    }
                }

                let mae = if total_pixels > 0 { sum_abs_diff / (total_pixels as f64) } else { 0.0 };
                let rmse = if total_pixels > 0 { (sum_sq_diff / (total_pixels as f64)).sqrt() } else { 0.0 };
                let match_pct = if total_pixels > 0 { (exact_matches as f64 / total_pixels as f64) * 100.0 } else { 0.0 };

                comparisons.push(CursorVisualComparison {
                    variant: variant.icon_name().to_string(),
                    scale,
                    background: bg_name.to_string(),
                    dimensions: [u32::from(w), u32::from(h)],
                    hotspot: [u32::from(hx), u32::from(hy)],
                    mae_rgb: mae,
                    max_abs_diff_rgb: max_diff,
                    rmse_rgb: rmse,
                    exact_match_pixels: exact_matches,
                    total_pixels,
                    match_percentage: match_pct,
                    pixels_diff_gt_2: diff_gt_2,
                    pixels_diff_gt_5: diff_gt_5,
                    pixels_diff_gt_15: diff_gt_15,
                });
            }
        }

        rendered_scale_images.push((scale, raw_pixels, img_w, img_h));
    }

    // Stitch rendered scales into a contact-sheet PNG
    let max_w = rendered_scale_images.iter().map(|(_, _, w, _)| *w).max().unwrap_or(0);
    let total_h: u32 = rendered_scale_images.iter().map(|(_, _, _, h)| *h + 10).sum();

    if max_w == 0 || total_h == 0 {
        return (comparisons, None);
    }

    let mut contact_sheet = vec![30u8; (max_w * total_h * 4) as usize];
    let mut curr_y = 0u32;
    for (_, pixels, w, h) in &rendered_scale_images {
        for row in 0..*h {
            let src_start = ((row * w) * 4) as usize;
            let src_end = src_start + (*w * 4) as usize;
            let dst_start = (((curr_y + row) * max_w) * 4) as usize;
            let dst_end = dst_start + (*w * 4) as usize;
            if dst_end <= contact_sheet.len() && src_end <= pixels.len() {
                contact_sheet[dst_start..dst_end].copy_from_slice(&pixels[src_start..src_end]);
            }
        }
        curr_y += *h + 10;
    }

    let img = match Image::from_raw(max_w, total_h, ChannelLayout::Rgba, SampleType::U8, contact_sheet) {
        Ok(i) => i,
        Err(_) => return (comparisons, None),
    };
    let encoded = match encode(&img, Format::Png, &EncodeOptions::default()) {
        Ok(e) => e,
        Err(_) => return (comparisons, None),
    };

    if let Some(dir) = save_dir {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join("pen_cursor_contact_sheet.png"), &encoded);
    }

    let b64 = base64::engine::general_purpose::STANDARD.encode(&encoded);
    (comparisons, Some(b64))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let moves: usize = arg_value(&args, "--moves").and_then(|v| v.parse().ok()).unwrap_or(1000);
    let reps: usize = arg_value(&args, "--reps").or_else(|| arg_value(&args, "--pairs")).and_then(|v| v.parse().ok()).unwrap_or(5);
    let save_dir = arg_value(&args, "--save-artifacts");

    if moves == 0 || moves > 100_000 {
        return Err("moves count must be in 1..=100000".into());
    }
    if reps == 0 || reps > 100 {
        return Err("reps count must be in 1..=100".into());
    }

    // Check for --json flag and optional path argument
    let json_pos = args.iter().position(|a| a == "--json");
    let json_file_path = json_pos.and_then(|i| args.get(i + 1).filter(|val| !val.starts_with("--")).cloned());
    let json_output = json_pos.is_some();

    let scales = [1.0f32, 1.25, 1.5, 2.0];
    let variants = PenVariant::all();

    // Warmup cycle to stabilize caches and CPU frequency scaling
    for variant in variants {
        for &scale in &scales {
            let _ = rasterize_pen(variant, scale);
        }
    }

    let mut results: Vec<BenchmarkRun> = Vec::new();
    let mut perf_rows: Vec<PerfReportRow> = Vec::new();
    let mut all_gates_passed = true;

    for variant in variants {
        let tool = tool_for_variant(variant);
        let variant_name = variant.icon_name();

        for &scale in &scales {
            // Context setup with native scale applied on all platforms
            let ctx = Context::default();
            egui_extras::install_image_loaders(&ctx);
            ctx.set_zoom_factor(scale);
            let at = pos2(100.0, 100.0);
            // Apply pending zoom during begin_pass before reading effective scale.
            let _ = cursor_frame(&ctx, tool, variant, at, false);
            let measured_scale = native_scale(&ctx);
            if (measured_scale - scale).abs() > 1e-4 {
                return Err(format!("Context scale mismatch for {scale}: measured {measured_scale}").into());
            }

            // 1. Cold rasterization measurements across repetitions
            let mut cold_samples = Vec::with_capacity(reps);
            let mut sample_image = None;
            for _ in 0..reps {
                let start_cold = Instant::now();
                let img = rasterize_pen(variant, scale).ok_or_else(|| format!("rasterize_pen failed for {variant:?} at {scale}"))?;
                let cold_us = start_cold.elapsed().as_secs_f64() * 1_000_000.0;
                cold_samples.push(cold_us);
                sample_image = Some(img);
            }
            let img = sample_image.ok_or("missing rasterized image")?;
            let [w, h] = img.size;
            let [hx, hy] = img.hotspot;
            if w == 0 || h == 0 || hx >= w || hy >= h {
                return Err(format!("invalid bitmap geometry for {variant:?} at {scale}: size {w}x{h}, hotspot {hx},{hy}").into());
            }

            // Verify expected dimensions at native scale
            let expected_pad = ((1.0 * scale) + 2.0).ceil() as usize;
            let icon_px = (22.0 * scale).round() as usize;
            let expected_side = icon_px + expected_pad * 2;
            if usize::from(w) != expected_side || usize::from(h) != expected_side {
                return Err(format!("unexpected dimensions for {variant:?} at scale {scale}: got {w}x{h}, expected {expected_side}x{expected_side}").into());
            }

            // 2. Warm movement / cache lookup measurement
            // Prime both paths, with actual frames and drained platform/paint output.
            let (_, initial_image) = cursor_frame(&ctx, tool, variant, at, false);
            let initial_image = initial_image.ok_or("missing native cursor in warmup frame")?;
            if initial_image.size != img.size {
                return Err("warm native bitmap does not match requested scale".into());
            }
            let _ = cursor_frame(&ctx, tool, variant, at, true);
            reset_pen_rasterization_count();
            let total_moves = moves.checked_mul(reps).ok_or("move count overflow")?;
            let mut warm_samples = Vec::with_capacity(total_moves);
            let mut baseline_samples = Vec::with_capacity(total_moves);
            let mut cache_hits = 0;
            // Alternate execution order by repetition; coordinates match for both paths.
            for rep in 0..reps {
                for step in 0..moves {
                    let p = pos2(100.0 + (step % 800) as f32 * 0.25, 100.0 + (step as f32 * 0.05).sin() * 40.0);
                    let mut native_image = None;
                    for painted in if rep % 2 == 0 { [false, true] } else { [true, false] } {
                        let (elapsed, image) = cursor_frame(&ctx, tool, variant, p, painted);
                        if painted {
                            baseline_samples.push(elapsed);
                        } else {
                            warm_samples.push(elapsed);
                            native_image = image;
                        }
                    }
                    let current = native_image.ok_or("native cursor missing from frame output")?;
                    if Arc::ptr_eq(&current.rgba, &initial_image.rgba) {
                        cache_hits += 1;
                    }
                }
            }
            let warm_rasterizations = pen_rasterization_count();

            let mut sorted_cold = cold_samples.clone();
            sorted_cold.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let cold_p50 = percentile(&sorted_cold, 0.50);
            let cold_p95 = percentile(&sorted_cold, 0.95);

            let mut sorted_warm = warm_samples.clone();
            sorted_warm.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let warm_p50 = percentile(&sorted_warm, 0.50);
            let warm_p95 = percentile(&sorted_warm, 0.95);

            let mut sorted_base = baseline_samples.clone();
            sorted_base.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let base_p50 = percentile(&sorted_base, 0.50);
            let base_p95 = percentile(&sorted_base, 0.95);

            let speedup = if warm_p50 > 0.0 { base_p50 / warm_p50 } else { 1.0 };

            // Structural gate: zero additional rasterizations during warm movement, 100% cache hits
            let structural_gate_pass = warm_rasterizations == 0 && cache_hits == total_moves;

            // Cost gate: warm-path p95 < 0.1 ms (100 µs)
            let cost_gate_pass = warm_p95 < 100.0;

            if !structural_gate_pass || !cost_gate_pass {
                all_gates_passed = false;
            }

            if let Some(ref dir_path) = save_dir {
                save_cursor_artifact(Path::new(dir_path), variant_name, scale, &img)?;
            }

            results.push(BenchmarkRun {
                variant: variant_name.to_string(),
                tool: format!("{tool:?}"),
                scale,
                measured_native_scale: measured_scale,
                dimensions: [w, h],
                hotspot: [hx, hy],
                retained_bytes: img.rgba.len(),
                cold_raster_us_samples: cold_samples,
                cold_raster_p50_us: cold_p50,
                cold_raster_p95_us: cold_p95,
                warm_move_us_samples: warm_samples.clone(),
                warm_move_p50_us: warm_p50,
                warm_move_p95_us: warm_p95,
                baseline_painted_us_samples: baseline_samples,
                baseline_painted_p50_us: base_p50,
                baseline_painted_p95_us: base_p95,
                preparation_speedup_vs_baseline_p50: speedup,
                warm_moves_total: total_moves,
                cache_hits,
                warm_rasterizations,
                structural_gate_pass,
                cost_gate_pass,
            });

            perf_rows.push(PerfReportRow {
                name: format!("pen_{variant_name}_{scale:.2}x"),
                samples_ms: warm_samples.iter().map(|us| us / 1000.0).collect(),
                p50_ms: warm_p50 / 1000.0,
                p95_ms: warm_p95 / 1000.0,
            });
        }
    }

    // Run visual consistency oracle and render contact sheet outside timing samples
    let (visual_comparisons, contact_sheet_b64) = run_visual_oracle(save_dir.as_deref().map(Path::new));

    if json_output {
        let perf_report = PerfReport {
            context: serde_json::json!({
                "bench": "cursor_bench",
                "moves": moves,
                "reps": reps,
                "all_gates_passed": all_gates_passed,
            }),
            rows: perf_rows,
            results: results.clone(),
            visual_comparisons: visual_comparisons.clone(),
            contact_sheet_png_base64: contact_sheet_b64,
        };
        let json_str = serde_json::to_string_pretty(&perf_report)?;
        if let Some(path) = json_file_path {
            std::fs::write(&path, json_str)?;
        } else {
            println!("{json_str}");
        }
    } else {
        println!("=================================================================================================================================");
        println!("PhotoCraft Issue #9122 Native Pen Cursor Benchmark Results");
        println!("Harness: {} pointer moves across {} repetitions per scale/variant", moves, reps);
        println!("=================================================================================================================================");
        println!(
            "{:<18} {:<6} {:<10} {:<10} {:<12} {:<12} {:<12} {:<14} {:<10} {:<8}",
            "Variant", "Scale", "Size (px)", "Hotspot", "Cold p50 (µs)", "Warm p50 (µs)", "Warm p95 (µs)", "Baseline p50", "Speedup", "Gates"
        );
        println!("---------------------------------------------------------------------------------------------------------------------------------");
        for r in &results {
            let gate_str = if r.structural_gate_pass && r.cost_gate_pass { "PASS" } else { "FAIL" };
            println!(
                "{:<18} {:<6.2} {:<10} {:<10} {:<12.1} {:<12.2} {:<12.2} {:<14.2} {:<9.1}x {:<8}",
                r.variant,
                r.scale,
                format!("{}x{}", r.dimensions[0], r.dimensions[1]),
                format!("{},{}", r.hotspot[0], r.hotspot[1]),
                r.cold_raster_p50_us,
                r.warm_move_p50_us,
                r.warm_move_p95_us,
                r.baseline_painted_p50_us,
                r.preparation_speedup_vs_baseline_p50,
                gate_str
            );
        }
        println!("=================================================================================================================================");
        println!("Measured Truth Gates Summary:");
        let total_warm_moves: usize = results.iter().map(|r| r.warm_moves_total).sum();
        let total_hits: usize = results.iter().map(|r| r.cache_hits).sum();
        let total_rasterizations: usize = results.iter().map(|r| r.warm_rasterizations).sum();
        println!(
            "- Structural gate (0 warm rasterizations, 100% cache hits): {} ({}/{} moves hit cache, {} re-rasterizations)",
            if results.iter().all(|r| r.structural_gate_pass) { "PASS" } else { "FAIL" },
            total_hits,
            total_warm_moves,
            total_rasterizations
        );
        println!("- Cost gate (warm-path p95 < 0.1 ms / 100 µs): {}", if results.iter().all(|r| r.cost_gate_pass) { "PASS" } else { "FAIL" });
        if let Some(ref dir_path) = save_dir {
            println!("- Cursor PNG artifacts saved to: {dir_path}");
        }
        if !visual_comparisons.is_empty() {
            println!("---------------------------------------------------------------------------------------------------------------------------------");
            println!("Visual Consistency Reference (Painted Baseline vs Native Bitmap 1:1 on wgpu):");
            println!(
                "{:<18} {:<6} {:<12} {:<10} {:<10} {:<10} {:<10} {:<10} {:<10}",
                "Variant", "Scale", "Background", "Size (px)", "Hotspot", "MAE (RGB)", "Max Diff", "RMSE", "Match %"
            );
            println!("---------------------------------------------------------------------------------------------------------------------------------");
            for vc in &visual_comparisons {
                println!(
                    "{:<18} {:<6.2} {:<12} {:<10} {:<10} {:<10.2} {:<10} {:<10.2} {:<9.1}%",
                    vc.variant,
                    vc.scale,
                    vc.background,
                    format!("{}x{}", vc.dimensions[0], vc.dimensions[1]),
                    format!("{},{}", vc.hotspot[0], vc.hotspot[1]),
                    vc.mae_rgb,
                    vc.max_abs_diff_rgb,
                    vc.rmse_rgb,
                    vc.match_percentage
                );
            }
        }
        println!("=================================================================================================================================");
    }

    if !all_gates_passed {
        return Err("one or more benchmark gates failed".into());
    }

    Ok(())
}
