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

use egui::{Context, pos2};
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType, encode};
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
    speedup_vs_baseline_p50: f64,
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
struct PerfReport {
    context: serde_json::Value,
    rows: Vec<PerfReportRow>,
    results: Vec<BenchmarkRun>,
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let moves: usize = arg_value(&args, "--moves").and_then(|v| v.parse().ok()).unwrap_or(1000);
    let reps: usize = arg_value(&args, "--reps").or_else(|| arg_value(&args, "--pairs")).and_then(|v| v.parse().ok()).unwrap_or(5);
    let save_dir = arg_value(&args, "--save-artifacts");

    if moves == 0 {
        return Err("moves count must be > 0".into());
    }
    if reps == 0 {
        return Err("reps count must be > 0".into());
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
            ctx.set_pixels_per_point(scale);
            ctx.set_zoom_factor(scale);
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
            reset_pen_rasterization_count();
            let initial_icon = pen(&ctx, tool).ok_or_else(|| format!("pen cursor unavailable for {variant:?} at {scale}"))?;
            if initial_icon != egui::CursorIcon::None {
                return Err(format!("expected CursorIcon::None from pen(), got {initial_icon:?}").into());
            }
            let initial_image = ctx.output(|o| o.cursor_image.clone()).ok_or("missing cursor_image in platform output")?;
            let rasterizations_before_moves = pen_rasterization_count();

            let total_moves = moves * reps;
            let mut warm_samples = Vec::with_capacity(total_moves);
            let mut cache_hits = 0;
            let mut missing_outputs = 0;

            for step in 0..total_moves {
                let start_move = Instant::now();
                let icon = pen(&ctx, tool);
                let move_us = start_move.elapsed().as_secs_f64() * 1_000_000.0;
                warm_samples.push(move_us);

                if icon != Some(egui::CursorIcon::None) {
                    return Err(format!("step {step}: pen returned unexpected icon {icon:?}").into());
                }

                let cur_img = ctx.output(|o| o.cursor_image.clone());
                match cur_img {
                    Some(ref cur) => {
                        if Arc::ptr_eq(&cur.rgba, &initial_image.rgba) {
                            cache_hits += 1;
                        }
                    }
                    None => {
                        missing_outputs += 1;
                    }
                }
            }

            if missing_outputs > 0 {
                return Err(format!("{missing_outputs} pointer moves had missing cursor_image platform output").into());
            }

            let rasterizations_after_moves = pen_rasterization_count();
            let warm_rasterizations = rasterizations_after_moves.saturating_sub(rasterizations_before_moves);

            // 3. Baseline: painted fallback measurement using identical movement coordinates
            let mut baseline_samples = Vec::with_capacity(total_moves);
            let hot = variant.hotspot_in_box() / 24.0;
            for step in 0..total_moves {
                let p = pos2(100.0 + (step as f32) * 0.25, 100.0 + ((step as f32) * 0.05).sin() * 40.0);
                let start_base = Instant::now();
                icons::cursor(&ctx, variant.icon_name(), p, hot, 22.0);
                let base_us = start_base.elapsed().as_secs_f64() * 1_000_000.0;
                baseline_samples.push(base_us);
            }

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
                speedup_vs_baseline_p50: speedup,
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
                r.speedup_vs_baseline_p50,
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
        println!("=================================================================================================================================");
    }

    if !all_gates_passed {
        return Err("one or more benchmark gates failed".into());
    }

    Ok(())
}
