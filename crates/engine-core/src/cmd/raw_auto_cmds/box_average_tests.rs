use super::{Rect, box_average};
use photocraft_color::PixelFormat;
use photocraft_raster::Surface;

/// The clamped reference: every block row read, past the last row the last row again.
fn reference(surf: &Surface, area: Rect, k: usize) -> Vec<[f32; 4]> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let (pw, ph) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = Vec::new();
    let mut row = vec![[0.0f32; 4]; w];
    for py in 0..ph {
        let mut acc = vec![[0.0f32; 5]; pw];
        for dy in 0..k {
            let y = area.y0 + (py * k + dy).min(h - 1) as i32;
            surf.read_rgba_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
            for (x, q) in row.iter().enumerate() {
                for c in 0..4 {
                    acc[x / k][c] += q[c];
                }
                acc[x / k][4] += 1.0;
            }
        }
        out.extend(acc.iter().map(|a| [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]]));
    }
    out
}

#[test]
fn blocks_past_the_last_row_match_the_clamped_average() {
    let mut surf = Surface::new(PixelFormat::RGBA32F);
    for x in 0..37 {
        for y in 0..5 {
            surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[x as f32 / 37.0, y as f32 / 5.0, 0.5, 1.0]);
        }
    }
    for (area, k) in [(Rect::new(0, 0, 37, 5), 2), (Rect::new(0, 0, 37, 5), 4), (Rect::new(0, 2, 37, 3), 16), (Rect::new(3, 0, 37, 5), 7)] {
        let (got, want) = (box_average(&surf, area, k), reference(&surf, area, k));
        assert_eq!(got.len(), want.len());
        for (g, w) in got.iter().zip(&want) {
            assert!(g.iter().zip(w).all(|(a, b)| (a - b).abs() < 1e-5), "{area:?} k={k}: {g:?} vs {w:?}");
        }
    }
}

/// Samples near f32::MAX stay finite: the weighted sums are held in f64.
#[test]
fn huge_samples_do_not_overflow() {
    let mut surf = Surface::new(PixelFormat::RGBA32F);
    let big = f32::MAX / 2.0;
    surf.fill_rect(Rect::new(0, 0, 2, 1), &[big, -big, big, 1.0]);
    let got = box_average(&surf, Rect::new(0, 0, 2, 1), 4);
    assert!(got.iter().flatten().all(|v| v.is_finite()), "{got:?}");
}

#[test]
fn transparent_pixels_do_not_darken_the_proxy() {
    let mut surf = Surface::new(PixelFormat::RGBA32F);
    surf.fill_rect(Rect::new(0, 0, 1, 1), &[0.8, 0.6, 0.4, 1.0]);
    let proxy = box_average(&surf, Rect::new(0, 0, 4, 1), 2);
    assert_eq!(proxy, [[0.8, 0.6, 0.4, 0.5], [0.0; 4]]);
}
