//! Relight at photo resolutions, including high bit depths and cancellation.
//!
//! cargo run --release -p photocraft-algo --example bench_relight -- --width 6000 --height 4000 --reps 9
//! Add --depth 16|32, --softness 1|25|100, or --cancel-after-ms 20.
//! Run the binary under /usr/bin/time -v to include peak process RSS. Fixture creation is
//! outside the timing; the deterministic native samples are the same at every revision.
use photocraft_algo::{FilterParams, apply_in_with};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let value = |key: &str| args.windows(2).find(|v| v[0] == key).map(|v| v[1].as_str());
    let width: i32 = value("--width").unwrap_or("6000").parse()?;
    let height: i32 = value("--height").unwrap_or("4000").parse()?;
    let depth: u8 = value("--depth").unwrap_or("8").parse()?;
    let softness: f32 = value("--softness").unwrap_or("25").parse()?;
    let reps: usize = value("--reps").unwrap_or("9").parse()?;
    let cancel_ms: Option<f64> = value("--cancel-after-ms").map(str::parse).transpose()?;
    if !(1..=16_000).contains(&width) || !(1..=16_000).contains(&height) || !(1..=100).contains(&reps) {
        return Err("width/height must be 1..16000 and reps 1..100".into());
    }
    let sample = match depth {
        8 => SampleType::U8,
        16 => SampleType::U16,
        32 => SampleType::F32,
        _ => return Err("depth must be 8, 16, or 32".into()),
    };
    let area = Rect::new(0, 0, width, height);
    let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
    for y0 in (0..height).step_by(256) {
        let end = (y0 + 256).min(height);
        let mut data = Vec::with_capacity(width as usize * (end - y0) as usize * 4);
        for y in y0..end {
            for x in 0..width {
                data.extend_from_slice(&[(x % 256) as f32 / 255.0, ((x * 3 + y) / 11 % 256) as f32 / 255.0, ((x ^ y) % 256) as f32 / 255.0, 1.0]);
            }
        }
        source.write_region(Rect::new(0, y0, width, end), &data);
    }
    let p = FilterParams::Relight { angle: 45.0, elevation: 40.0, intensity: 40.0, ambient: 55.0, warmth: 0.0, softness };
    let mut samples = Vec::with_capacity(reps);
    let mut cancelled = 0;
    let mut checksum = 0.0f64;
    for _ in 0..reps {
        let start = Instant::now();
        let cancel = || cancel_ms.is_some_and(|ms| start.elapsed().as_secs_f64() * 1000.0 >= ms);
        let out = apply_in_with(&source, &p, area, area, None, area, &Interrupt::cancel_only(&cancel));
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        match out {
            Some(out) => checksum += out.pixel(width / 2, height / 2).iter().map(|v| f64::from(*v)).sum::<f64>(),
            None => cancelled += 1,
        }
    }
    samples.sort_by(f64::total_cmp);
    let median = samples.get(samples.len() / 2).copied().unwrap_or(0.0);
    let p95 = samples.get((samples.len() * 95).div_ceil(100).saturating_sub(1)).copied().unwrap_or(0.0);
    println!(
        "{}",
        serde_json::json!({"width":width,"height":height,"depth":depth,"softness":softness,"reps":reps,"median_ms":median,"p95_ms":p95,"samples_ms":samples,"cancel_after_ms":cancel_ms,"cancelled":cancelled,"checksum":checksum})
    );
    Ok(())
}
