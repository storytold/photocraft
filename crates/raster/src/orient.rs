//! Exact right-angle turns and flips of a whole surface (Image › Image Rotation), tile by tile.
//!
//! Every target tile is built on its own (in parallel) by copying encoded pixels straight out of
//! the source tiles: no per-pixel decode/encode and no tile lookup per pixel, so flipping a large
//! document is about as fast as copying it once.

use std::collections::BTreeSet;
use std::sync::Arc;

use photocraft_geom::{TILE_SIZE, TileCoord};
use rayon::prelude::*;

use crate::{Surface, Tile};

/// A flip or right-angle turn of a `w × h` canvas, as pixel-index maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orient {
    FlipHorizontal,
    FlipVertical,
    Rotate180,
    Cw90,
    Ccw90,
}

/// Pixel coordinates in `i64`: a turn of content near the ends of the `i32` plane (a layer moved
/// far off the canvas) lands outside it, which `i32` arithmetic would overflow.
type P64 = (i64, i64);

/// `p` as a pixel of the `i32` plane, if it is one.
fn narrow((x, y): P64) -> Option<(i32, i32)> {
    Some((i32::try_from(x).ok()?, i32::try_from(y).ok()?))
}

impl Orient {
    fn map64(self, w: i64, h: i64, (x, y): P64) -> P64 {
        match self {
            Orient::FlipHorizontal => (w - 1 - x, y),
            Orient::FlipVertical => (x, h - 1 - y),
            Orient::Rotate180 => (w - 1 - x, h - 1 - y),
            Orient::Cw90 => (h - 1 - y, x),
            Orient::Ccw90 => (y, w - 1 - x),
        }
    }

    fn inverse64(self, w: i64, h: i64, (x, y): P64) -> P64 {
        match self {
            Orient::Cw90 => (y, h - 1 - x),
            Orient::Ccw90 => (w - 1 - y, x),
            // Flips and the half turn are their own inverse.
            o => o.map64(w, h, (x, y)),
        }
    }

    /// Where pixel `(x, y)` of a `w × h` canvas goes; `None` when that is off the `i32` plane.
    pub fn map(self, w: i32, h: i32, x: i32, y: i32) -> Option<(i32, i32)> {
        narrow(self.map64(w.into(), h.into(), (x.into(), y.into())))
    }

    /// The source pixel that [`Self::map`] sends to `(x, y)`; `None` when there is none on the
    /// `i32` plane.
    pub fn inverse(self, w: i32, h: i32, x: i32, y: i32) -> Option<(i32, i32)> {
        narrow(self.inverse64(w.into(), h.into(), (x.into(), y.into())))
    }

    /// The target tiles source tile `c` maps onto, within the `i32` plane (none when it lands
    /// entirely off it). Worked out in `i64` with inclusive bounds rather than through a
    /// [`Rect`](photocraft_geom::Rect), whose half-open end can't hold the pixel at `i32::MAX`.
    fn target_tiles(self, w: i32, h: i32, c: TileCoord) -> impl Iterator<Item = TileCoord> {
        let (w, h, ts) = (i64::from(w), i64::from(h), i64::from(TILE_SIZE));
        let (x0, y0) = (i64::from(c.tx) * ts, i64::from(c.ty) * ts);
        let (ax, ay) = self.map64(w, h, (x0, y0));
        let (bx, by) = self.map64(w, h, (x0 + ts - 1, y0 + ts - 1));
        let span = |a: i64, b: i64| {
            let (lo, hi) = (a.min(b).max(i32::MIN.into()), a.max(b).min(i32::MAX.into()));
            // Tile indices of the `i32` plane fit in `i32`.
            (lo.div_euclid(ts) as i32)..=(hi.div_euclid(ts) as i32)
        };
        let (xs, ys) = (span(ax, bx), span(ay, by));
        ys.flat_map(move |ty| xs.clone().map(move |tx| TileCoord::new(tx, ty)))
    }
}

impl Surface {
    /// This surface turned or flipped by `o` about a `w × h` canvas, pixel for pixel (pixels
    /// outside the canvas move with it). Target tiles left holding only the default pixel are
    /// not stored, so the result stays as sparse as the source.
    pub fn oriented(&self, o: Orient, w: i32, h: i32) -> Surface {
        let bpp = self.format.bytes_per_pixel();
        let ts = TILE_SIZE as usize;
        let dp: &[u8] = &self.default_pixel;
        // Only source tiles with content make target tiles: a large area written with the default
        // pixel (a cleared layer) can leave many allocated blank tiles, which would each cost a
        // target tile's allocation and scan for nothing. Gaps read as the default pixel anyway.
        let sources: Vec<TileCoord> = self.tiles.par_iter().filter(|(_, t)| t.data.chunks_exact(bpp).any(|px| px != dp)).map(|(c, _)| *c).collect();
        let targets: Vec<TileCoord> = sources.iter().flat_map(|&c| o.target_tiles(w, h, c)).collect::<BTreeSet<_>>().into_iter().collect();
        let (w64, h64) = (i64::from(w), i64::from(h));
        let tiles: Vec<(TileCoord, Arc<Tile>)> = targets
            .par_iter()
            .filter_map(|&dc| {
                // In `i64`: the last tile column's `Rect` stops short of the pixel at `i32::MAX`.
                let (ox, oy) = (i64::from(dc.tx) * i64::from(TILE_SIZE), i64::from(dc.ty) * i64::from(TILE_SIZE));
                let mut data = vec![0u8; ts * ts * bpp];
                // Consecutive target pixels nearly always come from the same source tile.
                let mut cached: Option<(TileCoord, Option<&Arc<Tile>>)> = None;
                let mut content = false;
                for (ly, row) in data.chunks_exact_mut(ts * bpp).enumerate() {
                    let y = oy + ly as i64;
                    for (lx, out) in row.chunks_exact_mut(bpp).enumerate() {
                        // A target pixel whose source would be off the `i32` plane has none.
                        let Some((sx, sy)) = narrow(o.inverse64(w64, h64, (ox + lx as i64, y))) else {
                            out.copy_from_slice(dp);
                            continue;
                        };
                        let sc = TileCoord::containing(sx, sy);
                        let src = match cached {
                            Some((c, t)) if c == sc => t,
                            _ => {
                                let t = self.tiles.get(&sc);
                                cached = Some((sc, t));
                                t
                            }
                        };
                        match src {
                            Some(t) => {
                                let i = (sy.rem_euclid(TILE_SIZE) as usize * ts + sx.rem_euclid(TILE_SIZE) as usize) * bpp;
                                let px = &t.data[i..i + bpp];
                                out.copy_from_slice(px);
                                content |= px != dp;
                            }
                            None => out.copy_from_slice(dp),
                        }
                    }
                }
                content.then(|| (dc, Arc::new(Tile { data: data.into_boxed_slice() })))
            })
            .collect();
        Surface { format: self.format, default_pixel: self.default_pixel.clone(), tiles: tiles.into_iter().collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_geom::Rect;

    /// The plain per-pixel remap this replaces (`photocraft_engine::pixels::remap_surface`).
    fn naive(s: &Surface, o: Orient, w: i32, h: i32) -> Surface {
        let r = s.content_bounds();
        let mut out = Surface::with_default(s.format(), &s.default_pixel());
        let mut px = vec![0.0; s.channels()];
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                s.read_pixel(x, y, &mut px);
                if let Some((nx, ny)) = o.map(w, h, x, y) {
                    out.write_pixel(nx, ny, &px);
                }
            }
        }
        out
    }

    fn assert_same(a: &Surface, b: &Surface, area: Rect) {
        let (mut pa, mut pb) = (vec![0.0; a.channels()], vec![0.0; b.channels()]);
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                a.read_pixel(x, y, &mut pa);
                b.read_pixel(x, y, &mut pb);
                assert_eq!(pa, pb, "pixel ({x}, {y})");
            }
        }
    }

    fn sample(format: PixelFormat, default: &[f32]) -> Surface {
        let mut s = Surface::with_default(format, default);
        let n = s.channels();
        // Content spanning tile seams, off the canvas on the left/top and past it on the right.
        for (x, y) in [(-3, -2), (0, 0), (255, 255), (256, 1), (300, 7), (511, 260), (612, 299)] {
            let px: Vec<f32> = (0..n).map(|c| ((x * 7 + y * 13 + c as i32 * 29).rem_euclid(255)) as f32 / 255.0).collect();
            s.write_pixel(x, y, &px);
        }
        let block: Vec<f32> = (0..40 * 30 * n).map(|i| (i % 251) as f32 / 255.0).collect();
        s.write_region(Rect::new(240, 240, 280, 270), &block);
        // A tile filled entirely with content (an opaque layer has nothing else), no default pixel.
        let full: Vec<f32> = (0..256 * 256 * n).map(|i| 0.1 + (i % 199) as f32 / 255.0).collect();
        s.write_region(Rect::new(512, 0, 768, 256), &full);
        s
    }

    #[test]
    fn matches_the_per_pixel_remap_for_every_turn() {
        let formats =
            [(PixelFormat::RGBA8, vec![0.0; 4]), (PixelFormat::new(ColorMode::Rgb, SampleType::U16, true), vec![0.0; 4]), (PixelFormat::GRAY8, vec![1.0])];
        // Odd sizes that aren't tile multiples, so target tiles straddle source seams.
        let (w, h) = (801, 300);
        for (format, default) in formats {
            let s = sample(format, &default);
            for o in [Orient::FlipHorizontal, Orient::FlipVertical, Orient::Rotate180, Orient::Cw90, Orient::Ccw90] {
                let fast = s.oriented(o, w, h);
                let slow = naive(&s, o, w, h);
                assert_same(&fast, &slow, Rect::new(-300, -300, 1100, 1100));
                assert_eq!(fast.content_bounds(), slow.content_bounds(), "{o:?}");
            }
        }
    }

    #[test]
    fn inverse_undoes_map() {
        let (w, h) = (37, 11);
        for o in [Orient::FlipHorizontal, Orient::FlipVertical, Orient::Rotate180, Orient::Cw90, Orient::Ccw90] {
            for (x, y) in [(0, 0), (36, 10), (5, 3), (-4, 20)] {
                let (mx, my) = o.map(w, h, x, y).unwrap();
                assert_eq!(o.inverse(w, h, mx, my), Some((x, y)), "{o:?}");
            }
        }
    }

    /// Content near the ends of the `i32` plane (a layer moved far off the canvas) turns without
    /// overflowing, quickly, to where the per-pixel map puts it, or is dropped when that is off
    /// the plane.
    #[test]
    fn content_at_the_ends_of_the_plane_turns_without_overflow() {
        let px = [0.2, 0.4, 0.6, 1.0];
        // Includes the very last pixels of the plane, and (on a 255 px canvas) pixels every turn
        // sends exactly to `i32::MAX` on one axis or both.
        let far = [
            (i32::MIN + 200, i32::MIN + 200),
            (i32::MAX - 200, i32::MAX - 200),
            (i32::MIN + 5, i32::MAX - 5),
            (i32::MAX - 1, i32::MIN),
            (i32::MAX, i32::MAX),
            (i32::MIN, i32::MIN),
            (i32::MAX, 0),
            (0, i32::MAX),
            (i32::MIN + 255, 0),
            (0, i32::MIN + 255),
            (i32::MIN + 255, i32::MIN + 255),
        ];
        for ((w, h), o) in [(100, 100), (255, 255)]
            .into_iter()
            .flat_map(|s| [Orient::FlipHorizontal, Orient::FlipVertical, Orient::Rotate180, Orient::Cw90, Orient::Ccw90].map(|o| (s, o)))
        {
            for (x, y) in far {
                let mut s = Surface::new(PixelFormat::RGBA8);
                s.write_pixel(x, y, &px);
                let t = std::time::Instant::now();
                let r = s.oriented(o, w, h);
                assert!(t.elapsed() < std::time::Duration::from_secs(2), "{o:?} ({x}, {y}) took {:?}", t.elapsed());
                match o.map(w, h, x, y) {
                    Some((nx, ny)) => {
                        assert_eq!(r.pixel(nx, ny), px.to_vec(), "{o:?} ({x}, {y})");
                        assert_eq!(r.tile_count(), 1, "{o:?} ({x}, {y})");
                    }
                    None => assert_eq!(r.tile_count(), 0, "{o:?} ({x}, {y}): lands off the plane"),
                }
            }
        }
        // The pixel CodeRabbit found: (i32::MIN + 255, 0) flips onto (i32::MAX, 0) on a 255 px canvas.
        assert_eq!(Orient::FlipHorizontal.map(255, 255, i32::MIN + 255, 0), Some((i32::MAX, 0)));
    }

    #[test]
    fn empty_and_default_tiles_stay_sparse() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        assert_eq!(s.oriented(Orient::FlipHorizontal, 100, 100).tile_count(), 0);
        // A tile allocated but holding only the default pixel is not carried over.
        let _ = s.tile_mut(TileCoord::new(0, 0));
        assert_eq!(s.oriented(Orient::Cw90, 100, 100).tile_count(), 0);
        // Among many blank tiles, only the one with content makes a target tile.
        for tx in 1..40 {
            let _ = s.tile_mut(TileCoord::new(tx, 3));
        }
        s.write_pixel(5, 5, &[1.0, 0.0, 0.0, 1.0]);
        let flipped = s.oriented(Orient::FlipHorizontal, 100, 100);
        assert_eq!(flipped.tile_count(), 1);
        assert_eq!(flipped.pixel(94, 5), vec![1.0, 0.0, 0.0, 1.0]);
    }
}
