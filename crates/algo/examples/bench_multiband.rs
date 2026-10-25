//! Time and peak memory of the Photomerge multi-band blend: `n` RGB sources of `mp` megapixels
//! (3:2), each overlapping the next by half its width.
//! `cargo run --release -p photocraft-algo --example bench_multiband [n] [mp]` (default 4 × 24 MP).
//! Peak memory is the growth of the process's high-water mark during the blend (Linux only).
use std::time::Instant;

use photocraft_algo::panorama::{self, RoiImage};

/// The process's peak resident set in MiB (`VmHWM`), where `/proc` has it.
fn peak_mib() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f64 = s.lines().find(|l| l.starts_with("VmHWM:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(4).max(1);
    let mp: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(24.0);
    let sh = ((mp * 1e6 / 1.5).sqrt().round() as usize).max(2);
    let sw = sh * 3 / 2;
    let step = sw / 2;
    let (cw, ch) = (step * (n - 1) + sw, sh);
    let rois: Vec<RoiImage> = (0..n)
        .map(|k| {
            let px = (0..sw * sh * 3).map(|i| ((i / 3 % sw * 7 + i / 3 / sw * 3 + k * 31 + i % 3 * 50) % 251) as f32 / 251.0).collect();
            RoiImage { x0: (step * k) as i32, y0: 0, w: sw, h: sh, ch: 3, px, alpha: vec![1.0; sw * sh] }
        })
        .collect();
    // Each canvas column belongs to the image whose centre is nearest, as a seam would split it.
    let wts: Vec<Vec<f32>> = (0..n)
        .map(|k| (0..sw * sh).map(|i| if ((step * k + i % sw + step / 2) / step).saturating_sub(1).min(n - 1) == k { 1.0 } else { 0.0 }).collect())
        .collect();
    let levels = panorama::blend_levels(cw, ch).min(7);
    let before = peak_mib();
    let t0 = Instant::now();
    let (px, cov) = panorama::multiband(cw, ch, &rois, &wts, levels);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let peak = peak_mib().zip(before).map_or("n/a".to_string(), |(a, b)| format!("{:.0} MiB", a - b));
    println!("{n} × {sw}×{sh} ({mp} MP) RGB, canvas {cw}×{ch}, {levels} levels: {ms:.0} ms, peak +{peak} during the blend");
    std::hint::black_box((px, cov));
}
