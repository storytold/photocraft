//! Pixel-art upscaling: Scale2x and Scale3x (the EPX family), from Andrea Mazzoleni's public
//! description of the rules (scale2x.it). Each source pixel becomes an n × n block that copies a
//! neighbour's colour wherever two neighbours that meet at that corner agree, so diagonal edges
//! step at the new resolution while flat areas and the palette stay exactly as they were: no
//! colour that isn't in the source appears, unlike any interpolating resample.
//!
//! Pixels are compared exactly on their normalised samples, so every depth and colour model
//! works the same (two pixels are equal when every channel, alpha included, is).

use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

/// The upscale to apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PixelArtScale {
    Scale2x,
    Scale3x,
    /// Scale2x run twice, as the reference implementation does for 4x.
    Scale4x,
}

impl PixelArtScale {
    /// How many times wider and taller the result is.
    pub fn factor(self) -> u32 {
        match self {
            PixelArtScale::Scale2x => 2,
            PixelArtScale::Scale3x => 3,
            PixelArtScale::Scale4x => 4,
        }
    }
}

/// Samples one band of source rows may hold (64 MB of `f32`), output included; larger surfaces
/// are upscaled in row bands.
const READ_BUDGET: usize = 1 << 24;

/// Upscales a layer, mask or channel of a document whose canvas is `canvas`, about the document
/// origin. Inside the canvas a neighbour past its edge counts as the pixel itself (the edge
/// repeats outwards, so a Background stays opaque to the border); pixels beyond the canvas use
/// their real neighbours. The surface's default pixel is kept.
///
/// Callers check that the scaled content bounds fit the `i32` coordinate range first.
pub fn upscale_surface_in_canvas(s: &Surface, scale: PixelArtScale, canvas: Rect) -> Surface {
    match scale {
        PixelArtScale::Scale2x => pass(s, 2, canvas, READ_BUDGET),
        PixelArtScale::Scale3x => pass(s, 3, canvas, READ_BUDGET),
        PixelArtScale::Scale4x => {
            let half = pass(s, 2, canvas, READ_BUDGET);
            pass(&half, 2, scale_rect(canvas, 2), READ_BUDGET)
        }
    }
}

/// The most memory [`upscale_surface_in_canvas`] holds at once for a `w` x `h` source of
/// `bytes_per_pixel` and `channels`, beyond the source itself: the result, Scale4x's 2x
/// intermediate (alive while the second pass writes the result), and one pass's `f32` band
/// buffers. Saturates instead of overflowing.
pub fn peak_bytes(scale: PixelArtScale, w: u32, h: u32, bytes_per_pixel: usize, channels: usize) -> u64 {
    let n = u64::from(scale.factor());
    let px = |k: u64| u64::from(w).saturating_mul(k).saturating_mul(u64::from(h).saturating_mul(k));
    let surfaces = match scale {
        PixelArtScale::Scale4x => px(4).saturating_add(px(2)),
        _ => px(n),
    }
    .saturating_mul(bytes_per_pixel as u64);
    // One band reads a row of the widest pass (with its margin) and writes its rows, never less
    // than one source row and otherwise capped at READ_BUDGET samples.
    let (src_w, pn) = match scale {
        PixelArtScale::Scale4x => (u64::from(w).saturating_mul(2), 2),
        _ => (u64::from(w), n),
    };
    let row = src_w.saturating_add(2).saturating_add(src_w.saturating_mul(pn * pn)).saturating_mul(channels.max(1) as u64);
    let band = row.max(READ_BUDGET as u64).saturating_mul(std::mem::size_of::<f32>() as u64);
    surfaces.saturating_add(band)
}

/// `r` scaled by `n` about the origin, saturating at the `i32` range.
pub fn scale_rect(r: Rect, n: u32) -> Rect {
    let n = n as i32;
    Rect::new(r.x0.saturating_mul(n), r.y0.saturating_mul(n), r.x1.saturating_mul(n), r.y1.saturating_mul(n))
}

/// One Scale2x (`n` = 2) or Scale3x (`n` = 3) pass.
fn pass(s: &Surface, n: u32, canvas: Rect, budget: usize) -> Surface {
    let def = s.default_pixel();
    let mut out = Surface::with_default(s.format(), &def);
    let src = s.content_bounds();
    if src.is_empty() {
        return out;
    }
    let ch = s.channels().max(1);
    let n_us = n as usize;
    let w = src.width() as usize;
    // The read window is one pixel wider on every side, for the neighbours.
    let rw = w + 2;
    let per_row = (rw + w * n_us * n_us) * ch;
    let band = (budget / per_row.max(1)).max(1) as i32;
    let mut y0 = src.y0;
    while y0 < src.y1 {
        let y1 = y0.saturating_add(band).min(src.y1);
        let read = Rect::new(src.x0.saturating_sub(1), y0.saturating_sub(1), src.x1.saturating_add(1), y1.saturating_add(1));
        let px = s.read_region(read);
        let rows = (y1 - y0) as usize;
        let ow = w * n_us;
        let mut dst = vec![0.0f32; ow * rows * n_us * ch];
        let ctx = Band { px: &px, read, rw, ch, canvas };
        let fill = |i: usize, block: &mut [f32]| ctx.row(y0 + i as i32, src.x0, src.x1, n_us, ow, block);
        crate::par::chunks_mut(&mut dst, ow * n_us * ch, true, fill);
        out.write_region(scale_rect(Rect::new(src.x0, y0, src.x1, y1), n), &dst);
        y0 = y1;
    }
    out.prune();
    out
}

/// One band of source pixels, read with a one-pixel margin.
struct Band<'a> {
    px: &'a [f32],
    read: Rect,
    rw: usize,
    ch: usize,
    canvas: Rect,
}

impl Band<'_> {
    /// The pixel at document `(x, y)`, which lies inside the read window.
    fn at(&self, x: i32, y: i32) -> &[f32] {
        let i = (y - self.read.y0) as usize * self.rw + (x - self.read.x0) as usize;
        self.px.get(i * self.ch..(i + 1) * self.ch).unwrap_or(&[])
    }

    /// The neighbour of `(x, y)` at `(x + dx, y + dy)`: inside the canvas, an axis that would
    /// leave it stays on the pixel's own row or column.
    fn near(&self, x: i32, y: i32, dx: i32, dy: i32) -> &[f32] {
        let c = &self.canvas;
        let edge = |p: i32, d: i32, lo: i32, hi: i32| if (lo..hi).contains(&p) && !(lo..hi).contains(&(p + d)) { p } else { p + d };
        self.at(edge(x, dx, c.x0, c.x1), edge(y, dy, c.y0, c.y1))
    }

    /// The `n` output rows of source row `y` (`block` holds them, `ow` pixels each).
    fn row(&self, y: i32, x0: i32, x1: i32, n: usize, ow: usize, block: &mut [f32]) {
        let ch = self.ch;
        for x in x0..x1 {
            let p = |dx, dy| self.near(x, y, dx, dy);
            let (a, b, c) = (p(-1, -1), p(0, -1), p(1, -1));
            let (d, e, f) = (p(-1, 0), p(0, 0), p(1, 0));
            let (g, h, i) = (p(-1, 1), p(0, 1), p(1, 1));
            let mut cells: [&[f32]; 9] = [e; 9];
            // No edge runs through the pixel when its opposite neighbours agree on either axis.
            if b != h && d != f {
                if n == 2 {
                    cells[0] = if d == b { d } else { e };
                    cells[1] = if b == f { f } else { e };
                    cells[2] = if d == h { d } else { e };
                    cells[3] = if h == f { f } else { e };
                } else {
                    cells[0] = if d == b { d } else { e };
                    cells[1] = if (d == b && e != c) || (b == f && e != a) { b } else { e };
                    cells[2] = if b == f { f } else { e };
                    cells[3] = if (d == b && e != g) || (d == h && e != a) { d } else { e };
                    cells[5] = if (b == f && e != i) || (h == f && e != c) { f } else { e };
                    cells[6] = if d == h { d } else { e };
                    cells[7] = if (d == h && e != i) || (h == f && e != g) { h } else { e };
                    cells[8] = if h == f { f } else { e };
                }
            }
            let ox = (x - x0) as usize * n;
            for (k, cell) in cells.iter().take(n * n).enumerate() {
                let o = ((k / n) * ow + ox + k % n) * ch;
                if let (Some(slot), Some(px)) = (block.get_mut(o..o + ch), cell.get(..ch)) {
                    slot.copy_from_slice(px);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};

    const K: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const W: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    const R: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

    fn surface(sample: SampleType, rows: &[&[[f32; 4]]]) -> Surface {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        for (y, row) in rows.iter().enumerate() {
            for (x, p) in row.iter().enumerate() {
                s.write_pixel(x as i32, y as i32, p);
            }
        }
        s
    }

    fn canvas(rows: &[&[[f32; 4]]]) -> Rect {
        Rect::new(0, 0, rows[0].len() as i32, rows.len() as i32)
    }

    /// A black staircase under a white sky.
    const STAIRS: [&[[f32; 4]]; 3] = [&[K, W, W], &[K, K, W], &[K, K, K]];

    #[test]
    fn scale2x_rounds_the_staircase_corner_with_existing_colours() {
        let s = surface(SampleType::U8, &STAIRS);
        let out = upscale_surface_in_canvas(&s, PixelArtScale::Scale2x, canvas(&STAIRS));
        assert_eq!(out.content_bounds(), Rect::new(0, 0, 6, 6));
        // The middle pixel's top-right quarter takes the sky, the rest stays black.
        assert_eq!(out.rgba(3, 2), W);
        for (x, y) in [(2, 2), (2, 3), (3, 3)] {
            assert_eq!(out.rgba(x, y), K, "({x}, {y})");
        }
        // Nearest Neighbor would have left the whole 2 × 2 block black.
        assert_eq!(out.rgba(1, 1), K);
    }

    #[test]
    fn scale3x_softens_only_the_outer_corner() {
        let s = surface(SampleType::U8, &STAIRS);
        let out = upscale_surface_in_canvas(&s, PixelArtScale::Scale3x, canvas(&STAIRS));
        assert_eq!(out.content_bounds(), Rect::new(0, 0, 9, 9));
        for y in 3..6 {
            for x in 3..6 {
                let want = if (x, y) == (5, 3) { W } else { K };
                assert_eq!(out.rgba(x, y), want, "({x}, {y})");
            }
        }
    }

    #[test]
    fn scale4x_is_scale2x_twice() {
        let s = surface(SampleType::U8, &STAIRS);
        let c = canvas(&STAIRS);
        let twice = upscale_surface_in_canvas(&upscale_surface_in_canvas(&s, PixelArtScale::Scale2x, c), PixelArtScale::Scale2x, scale_rect(c, 2));
        let out = upscale_surface_in_canvas(&s, PixelArtScale::Scale4x, c);
        assert_eq!(out.content_bounds(), Rect::new(0, 0, 12, 12));
        assert_eq!(out.read_region(Rect::new(0, 0, 12, 12)), twice.read_region(Rect::new(0, 0, 12, 12)));
    }

    #[test]
    fn flat_areas_and_the_palette_stay_exact_at_every_depth() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            // A pseudo-random three-colour image.
            let rows: Vec<Vec<[f32; 4]>> = (0..9).map(|y| (0..11).map(|x| [K, W, R][(x * 7 + y * 3 + x * y) % 3]).collect()).collect();
            let refs: Vec<&[[f32; 4]]> = rows.iter().map(Vec::as_slice).collect();
            let s = surface(sample, &refs);
            for scale in [PixelArtScale::Scale2x, PixelArtScale::Scale3x, PixelArtScale::Scale4x] {
                let out = upscale_surface_in_canvas(&s, scale, canvas(&refs));
                let n = scale.factor() as i32;
                assert_eq!(out.content_bounds(), Rect::new(0, 0, 11 * n, 9 * n), "{sample:?} {scale:?}");
                for y in 0..9 * n {
                    for x in 0..11 * n {
                        let p = out.rgba(x, y);
                        assert!([K, W, R].contains(&p), "{sample:?} {scale:?} ({x}, {y}) is {p:?}, not a source colour");
                    }
                }
            }
            let flat = surface(sample, &[&[R, R], &[R, R]]);
            let out = upscale_surface_in_canvas(&flat, PixelArtScale::Scale3x, Rect::new(0, 0, 2, 2));
            assert!(out.read_region(Rect::new(0, 0, 6, 6)).chunks(4).all(|p| p == R), "{sample:?}");
        }
    }

    #[test]
    fn row_bands_match_one_read() {
        let rows: Vec<Vec<[f32; 4]>> = (0..23).map(|y| (0..17).map(|x| if (x + y) % 5 < 2 || x == y { K } else { W }).collect()).collect();
        let refs: Vec<&[[f32; 4]]> = rows.iter().map(Vec::as_slice).collect();
        let s = surface(SampleType::U8, &refs);
        for n in [2, 3] {
            let whole = pass(&s, n, canvas(&refs), READ_BUDGET);
            let banded = pass(&s, n, canvas(&refs), 1);
            let r = scale_rect(canvas(&refs), n);
            assert_eq!(whole.read_region(r), banded.read_region(r), "{n}x");
        }
    }

    #[test]
    fn canvas_edges_repeat_so_an_opaque_layer_stays_opaque() {
        // Without the edge rule the transparent default past the top-left corner would match on
        // both sides of the corner and cut it away.
        let rows: [&[[f32; 4]]; 2] = [&[R, W], &[R, W]];
        let s = surface(SampleType::U8, &rows);
        for scale in [PixelArtScale::Scale2x, PixelArtScale::Scale3x, PixelArtScale::Scale4x] {
            let out = upscale_surface_in_canvas(&s, scale, canvas(&rows));
            let n = scale.factor() as i32;
            for y in 0..2 * n {
                for x in 0..2 * n {
                    assert_eq!(out.rgba(x, y), if x < n { R } else { W }, "{scale:?} ({x}, {y})");
                }
            }
        }
        // Past the canvas the real (transparent) neighbours count: a lone pixel off-canvas gets
        // no corners repeated into it but stays one solid block.
        let mut lone = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        lone.write_pixel(-3, -3, &R);
        let out = upscale_surface_in_canvas(&lone, PixelArtScale::Scale2x, Rect::new(0, 0, 4, 4));
        assert_eq!(out.content_bounds(), Rect::new(-6, -6, -4, -4));
        assert_eq!(out.read_region(Rect::new(-6, -6, -4, -4)).chunks(4).filter(|p| *p == R).count(), 4);
    }

    #[test]
    fn peak_bytes_counts_the_result_the_scale4x_intermediate_and_the_bands() {
        let band = READ_BUDGET as u64 * 4;
        assert_eq!(peak_bytes(PixelArtScale::Scale2x, 10, 10, 4, 4), 400 * 4 + band);
        assert_eq!(peak_bytes(PixelArtScale::Scale3x, 10, 10, 4, 4), 900 * 4 + band);
        // Scale4x keeps the 2x image (a quarter of the result) while it writes the result.
        assert_eq!(peak_bytes(PixelArtScale::Scale4x, 10, 10, 4, 4), (1600 + 400) * 4 + band);
        // A row wider than the budget is still read whole.
        let wide = 1 << 22;
        assert_eq!(peak_bytes(PixelArtScale::Scale2x, wide, 1, 1, 1) - 4 * u64::from(wide), (u64::from(wide) * 5 + 2) * 4);
        assert_eq!(peak_bytes(PixelArtScale::Scale4x, u32::MAX, u32::MAX, 16, 5), u64::MAX);
    }

    #[test]
    fn empty_and_mask_surfaces_keep_their_default_pixel() {
        let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
        let mask = Surface::with_default(gray, &[1.0]);
        let out = upscale_surface_in_canvas(&mask, PixelArtScale::Scale3x, Rect::new(0, 0, 8, 8));
        assert!(out.content_bounds().is_empty());
        assert_eq!(out.pixel(5, 5), vec![1.0]);
        let mut hole = Surface::with_default(gray, &[1.0]);
        hole.write_pixel(1, 1, &[0.0]);
        let out = upscale_surface_in_canvas(&hole, PixelArtScale::Scale2x, Rect::new(0, 0, 4, 4));
        assert_eq!((out.pixel(2, 2), out.pixel(3, 3), out.pixel(4, 4)), (vec![0.0], vec![0.0], vec![1.0]));
    }
}
