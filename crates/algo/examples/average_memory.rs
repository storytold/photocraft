//! Release benchmark of Average's scratch memory. Run each mode in a fresh process and
//! record its peak working set externally: `cargo run -p photocraft-algo --release
//! --example average_memory -- stream` (or `dense`). Append `selected` for a partial mask.
//! Both hash all output samples.
use std::{collections::hash_map::DefaultHasher, hash::Hasher, time::Instant};

use photocraft_algo::{FilterParams, apply};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into, to_rgba};

// The engine's map_pixels workspace before Average moved to algo, with the transparency fix.
// Keep all four buffers alive through writeback, just as map_pixels did.
fn dense(surface: &Surface, area: Rect, selection: Option<&Surface>) -> Surface {
    let fmt = surface.format();
    let n = fmt.channels();
    let mut raw = surface.read_region(area);
    let orig: Vec<_> = raw.chunks_exact(n).map(|p| to_rgba(&fmt, p)).collect();
    let width = area.width() as usize;
    let cover: Vec<_> =
        (0..orig.len()).map(|i| selection.map_or(1.0, |s| s.sample_channel(area.x0 + (i % width) as i32, area.y0 + (i / width) as i32, 0))).collect();
    let mut px = orig.clone();
    let mut sum = [0.0f64; 3];
    let mut weight = 0.0f64;
    for (p, &k) in px.iter().zip(&cover) {
        for c in 0..3 {
            sum[c] += f64::from(p[c]) * f64::from(k);
        }
        weight += f64::from(k);
    }
    let avg = sum.map(|s| (s / weight) as f32);
    for p in &mut px {
        p[..3].copy_from_slice(&avg);
    }
    for (i, native) in raw.chunks_exact_mut(n).enumerate() {
        let k = cover[i];
        if k <= 0.0 {
            continue;
        }
        let rgba = std::array::from_fn(|c| orig[i][c] + (px[i][c] - orig[i][c]) * k);
        from_rgba_into(&fmt, rgba, native);
    }
    let mut out = surface.clone();
    out.write_region(area, &raw);
    out.prune();
    std::hint::black_box((&orig, &cover, &px));
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "stream".into());
    let selected = std::env::args().nth(2).is_some_and(|s| s == "selected");
    if mode != "stream" && mode != "dense" {
        return Err("expected stream or dense".into());
    }
    let area = Rect::new(0, 0, 6000, 4000);
    let mut source = Surface::new(PixelFormat::RGBA8);
    let row: Vec<f32> = (0..6000).flat_map(|x| if x < 3000 { [1.0, 0.0, 0.0, 0.25] } else { [0.0, 0.0, 1.0, 0.75] }).collect();
    for y in 0..4000 {
        source.write_region(Rect::new(0, y, 6000, y + 1), &row);
    }
    let selection = selected.then(|| {
        let mut mask = Surface::new(PixelFormat::GRAY8);
        mask.fill_rect(area, &[0.5]);
        mask.fill_rect(Rect::new(1500, 0, 3000, 4000), &[0.0]);
        mask.fill_rect(Rect::new(4500, 0, 6000, 4000), &[1.0]);
        mask
    });
    let start = Instant::now();
    let out = if mode == "dense" { dense(&source, area, selection.as_ref()) } else { apply(&source, &FilterParams::Average, area, area, selection.as_ref()) };
    let elapsed = start.elapsed().as_secs_f64();
    let mut hash = DefaultHasher::new();
    for y in 0..4000 {
        for v in out.read_region(Rect::new(0, y, 6000, y + 1)) {
            hash.write_u32(v.to_bits());
        }
    }
    println!("mode={mode} selected={selected} pixels=24000000 seconds={elapsed:.6} hash={:016x}", hash.finish());
    Ok(())
}
