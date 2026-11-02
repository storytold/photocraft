//! Benchmark example measuring headless CPU vs GPU rendering and export performance.
//!
//! Usage:
//! ```sh
//! cargo run --release -p photocraft-cli --example gpu_bench -- [options]
//! ```

use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use photocraft_automation::files;
use photocraft_automation::gpu::GpuRenderer;
use photocraft_doc::{BlendMode, Color, ColorMode, Document, Layer, LayerContent, Rect, SampleType, Size, Surface};
use photocraft_io::ExportOptions;

#[derive(Parser, Debug)]
#[command(name = "gpu_bench", about = "Benchmark headless CPU vs GPU rendering and export performance")]
struct BenchArgs {
    /// Input image or PSD file (optional; defaults to synthetic mockup)
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Synthetic document width in pixels (1..=16384)
    #[arg(short, long, default_value_t = 2048, value_parser = clap::value_parser!(u32).range(1..=16384))]
    width: u32,

    /// Synthetic document height in pixels (1..=16384)
    #[arg(short = 'H', long, default_value_t = 2048, value_parser = clap::value_parser!(u32).range(1..=16384))]
    height: u32,

    /// Synthetic layer count (1..=128)
    #[arg(short, long, default_value_t = 8, value_parser = clap::value_parser!(u32).range(1..=128))]
    layers: u32,

    /// Number of warm GPU runs (0..=50)
    #[arg(short = 'r', long = "warm-runs", default_value_t = 3, value_parser = clap::value_parser!(u32).range(0..=50))]
    warm_runs: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = BenchArgs::parse();
    let layers = args.layers as usize;
    let warm_runs = args.warm_runs as usize;

    println!("# PhotoCraft CLI Headless Rendering Benchmark");
    println!();

    const MAX_SYNTHETIC_PIXELS: u64 = 256 * 1024 * 1024;
    let doc = if let Some(path) = &args.input {
        println!("- **Input Source**: file `{}`", path.display());
        let open_res = files::open(path)?;
        open_res.document
    } else {
        let total_pixels = u64::from(args.width) * u64::from(args.height) * u64::from(args.layers);
        if total_pixels > MAX_SYNTHETIC_PIXELS {
            return Err(format!(
                "requested synthetic benchmark volume ({}x{} x {} layers = {} total pixels) exceeds budget of {} pixels (~{} MB); please lower width, height, or layer count",
                args.width,
                args.height,
                args.layers,
                total_pixels,
                MAX_SYNTHETIC_PIXELS,
                (MAX_SYNTHETIC_PIXELS * 4) / (1024 * 1024)
            )
            .into());
        }
        println!("- **Input Source**: synthetic mockup ({}x{}, {} layers)", args.width, args.height, args.layers);
        sample_doc(args.width, args.height, layers)
    };

    println!("- **Dimensions**: {}x{}", doc.size.width, doc.size.height);
    println!("- **Layer Count**: {}", doc.layers.len());
    println!();

    let tmp_dir = std::env::temp_dir().join(format!("pc-bench-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let cpu_out = tmp_dir.join("cpu_out.png");
    let gpu_cold_out = tmp_dir.join("gpu_cold_out.png");
    let gpu_warm_out = tmp_dir.join("gpu_warm_out.png");

    let opts = ExportOptions::default();

    // 1. CPU Run
    print!("Benchmarking CPU export... ");
    let t0 = Instant::now();
    files::save_with_renderer(&doc, &cpu_out, Some("png"), &opts, None, None)?;
    let cpu_dur = t0.elapsed();
    println!("{:.2} ms", cpu_dur.as_secs_f64() * 1000.0);

    // 2. GPU Cold Run
    print!("Benchmarking GPU cold export... ");
    let mut gpu_renderer = GpuRenderer::lazy();
    let t0 = Instant::now();
    let gpu_cold_res = files::save_with_renderer(&doc, &gpu_cold_out, Some("png"), &opts, None, Some(&mut gpu_renderer));
    let gpu_cold_dur = t0.elapsed();

    let (gpu_succeeded, is_fallback) = match gpu_cold_res {
        Ok(warnings) => {
            let fallback = warnings.iter().any(|w| w.contains("falling back to CPU"));
            if fallback {
                println!("{:.2} ms (FALLBACK TO CPU: {:?})", gpu_cold_dur.as_secs_f64() * 1000.0, warnings);
                (true, true)
            } else {
                println!("{:.2} ms", gpu_cold_dur.as_secs_f64() * 1000.0);
                let adapter = gpu_renderer.adapter_name().unwrap_or("unknown");
                let is_soft = if gpu_renderer.is_software() { " (software rasterizer)" } else { " (hardware)" };
                println!("- **GPU Adapter**: `{adapter}`{is_soft}");
                (true, false)
            }
        }
        Err(e) => {
            println!("FAILED ({e})");
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Ok(());
        }
    };

    // 3. GPU Warm Runs
    let mut warm_durations = Vec::with_capacity(warm_runs);
    if gpu_succeeded && !is_fallback && warm_runs > 0 {
        for run in 1..=warm_runs {
            print!("Benchmarking GPU warm export (run {run}/{warm_runs})... ");
            let t0 = Instant::now();
            let warm_res = files::save_with_renderer(&doc, &gpu_warm_out, Some("png"), &opts, None, Some(&mut gpu_renderer))?;
            let d = t0.elapsed();
            if warm_res.iter().any(|w| w.contains("falling back to CPU")) {
                println!("{:.2} ms (FALLBACK TO CPU)", d.as_secs_f64() * 1000.0);
            } else {
                println!("{:.2} ms", d.as_secs_f64() * 1000.0);
                warm_durations.push(d);
            }
        }
    }

    println!();
    println!("### Summary Results");
    println!("| Mode | Duration | Relative to CPU | Note |");
    println!("|---|---|---|---|");
    println!("| CPU Baseline | {:.2} ms | 1.00x | Reference CPU compositor |", cpu_dur.as_secs_f64() * 1000.0);
    if is_fallback {
        println!("| GPU Cold | {:.2} ms | N/A | Fallback to CPU occurred; no GPU speedup |", gpu_cold_dur.as_secs_f64() * 1000.0);
        println!("| GPU Warm | N/A | N/A | Not measured (GPU fell back to CPU) |");
    } else {
        let is_software = gpu_renderer.is_software();
        let adapter_note = if is_software { "Software rasterizer (e.g. lavapipe/llvmpipe)" } else { "Hardware GPU" };
        println!(
            "| GPU Cold | {:.2} ms | {:.2}x | Cold adapter initialization + pipeline build ({adapter_note}) |",
            gpu_cold_dur.as_secs_f64() * 1000.0,
            cpu_dur.as_secs_f64() / gpu_cold_dur.as_secs_f64()
        );
        if !warm_durations.is_empty() {
            let avg_warm: f64 = warm_durations.iter().map(|d| d.as_secs_f64()).sum::<f64>() / (warm_durations.len() as f64);
            println!(
                "| GPU Warm (avg) | {:.2} ms | {:.2}x | Reused device context and pipeline caches ({adapter_note}) |",
                avg_warm * 1000.0,
                cpu_dur.as_secs_f64() / avg_warm
            );
        } else {
            println!("| GPU Warm | N/A | N/A | Not measured (0 warm runs requested) |");
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok(())
}

fn sample_doc(w: u32, h: u32, layer_count: usize) -> Document {
    let mut doc = Document::with_background("bench", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let blend_modes = [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay, BlendMode::Darken, BlendMode::Lighten];

    for idx in 0..layer_count {
        let mut s = Surface::new(doc.pixel_format());
        let x0 = (idx as i32 * 20) % (w as i32 / 2);
        let y0 = (idx as i32 * 20) % (h as i32 / 2);
        let rw = (w as i32 - x0).max(1);
        let rh = (h as i32 - y0).max(1);
        s.fill_rect(Rect::new(x0, y0, x0 + rw, y0 + rh), &[0.3, 0.5, 0.7, 0.8]);
        let mut layer = Layer::new(&format!("Layer {}", idx + 1), LayerContent::Raster(s));
        layer.opacity = 0.85;
        layer.blend = blend_modes[idx % blend_modes.len()];
        doc.layers.push(layer);
    }

    doc
}
