//! Timings for Minimum / Maximum: one timed run per invocation (so before/after binaries can be
//! run in alternating pairs), the complete output written as little-endian f32 sample bits for `cmp`.
//!
//! `cargo run --release -p photocraft-algo --example bench_min_max -- <min|max> <round|square> <radius> [out.bin]`
//! on a 6000×4000 RGBA8 photo-like layer with soft alpha.
use photocraft_algo::{FilterParams, Preserve, apply_in, output_area};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let max = args.get(1).is_some_and(|a| a == "max");
    let preserve = if args.get(2).is_some_and(|a| a == "square") { Preserve::Squareness } else { Preserve::Roundness };
    let radius: f32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(10.0);
    let (w, h) = (6000usize, 4000usize);
    let mut px = vec![0.0f32; w * h * 4];
    for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let n = ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 997) as f32 / 997.0 - 0.5;
        let inside = (x as f32 - 3300.0).hypot(y as f32 - 2000.0) < 1300.0;
        let base = if inside { 0.8 } else { 0.35 };
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0;
        *p = [q(base + 0.1 * n), q(0.45 + 0.08 * n), q(0.5 - 0.1 * n), if x % 97 < 3 { 0.5 } else { 1.0 }];
    }
    let r = Rect::new(0, 0, w as i32, h as i32);
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.write_region(r, &px);
    let p = if max { FilterParams::Maximum { radius, preserve } } else { FilterParams::Minimum { radius, preserve } };
    let area = output_area(&p, s.content_bounds(), r, None).intersect(&r);
    let t = Instant::now();
    let out = apply_in(&s, &p, area, r, None, r);
    println!("{} {preserve:?} r {radius}: {:.1} ms", if max { "max" } else { "min" }, t.elapsed().as_secs_f64() * 1e3);
    if let Some(path) = args.get(4) {
        let bytes: Vec<u8> = out.read_region(r).iter().flat_map(|v| v.to_bits().to_le_bytes()).collect();
        if let Err(e) = std::fs::write(path, bytes) {
            eprintln!("write {path}: {e}");
            std::process::exit(1);
        }
    }
}
