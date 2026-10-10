use std::time::Instant;

use photocraft_paint::brush::{BrushSettings, TipShape};
use photocraft_paint::outline::tip_outline;
use photocraft_paint::tile::GrayTile;

fn rng(seed: u64) -> impl FnMut() -> f32 {
    let mut s = seed;
    move || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((s >> 33) as f32) / ((1u64 << 31) as f32)
    }
}

fn main() {
    // Soft disc 64: the cheap end (one smooth ring).
    let disc = GrayTile::from_fn(64, 64, |x, y| {
        let (dx, dy) = (x as f32 - 31.5, y as f32 - 31.5);
        (1.0 - (dx * dx + dy * dy).sqrt() / 32.0).clamp(0.0, 1.0)
    });
    // Spatter 96: many islands, worst realistic case.
    let mut r = rng(7);
    let mut dots: Vec<(f32, f32, f32)> = (0..40).map(|_| (r() * 96.0, r() * 96.0, 2.0 + r() * 8.0)).collect();
    dots.push((48.0, 48.0, 20.0));
    let spatter = GrayTile::from_fn(96, 96, |x, y| {
        let mut v = 0.0f32;
        for &(cx, cy, cr) in &dots {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            v = v.max((1.0 - d / cr).clamp(0.0, 1.0));
        }
        v
    });
    // Big bitmap 512: a stored tip loaded at full size.
    let big = GrayTile::from_fn(512, 512, |x, y| {
        let (dx, dy) = (x as f32 - 255.5, y as f32 - 255.5);
        let n = ((x * 13 + y * 7) % 5) as f32 / 5.0;
        ((1.0 - (dx * dx + dy * dy).sqrt() / 256.0).clamp(0.0, 1.0) * (0.5 + 0.5 * n)).clamp(0.0, 1.0)
    });
    for (name, tile, size) in [("soft disc 64", disc, 60.0f32), ("spatter 96", spatter, 300.0), ("big 512", big, 800.0)] {
        for full in [false, true] {
            let mut b = BrushSettings { size, ..Default::default() };
            b.tip = TipShape::Sampled(tile.clone());
            let _ = tip_outline(&b, full);
            let n = 200;
            let t = Instant::now();
            let mut pts = 0usize;
            let mut rings = 0usize;
            for i in 0..n {
                if i % 2 == 0 {
                    b.angle = (i as f32) * 0.01; // defeat any future internal cache
                }
                if let Some(r) = tip_outline(&b, full) {
                    rings = r.len();
                    pts = r.iter().map(Vec::len).sum();
                }
            }
            let us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
            println!("{name} full={full}: {us:8.1} us/call   rings={rings} points={pts}");
        }
    }
    // Round tips: the ellipse path.
    let e = BrushSettings { size: 300.0, angle: 33.0, roundness: 0.4, ..Default::default() };
    let t = Instant::now();
    for _ in 0..1000 {
        let _ = tip_outline(&e, true);
    }
    println!("round ellipse: {:.2} us/call", t.elapsed().as_secs_f64() * 1e6 / 1000.0);
}
