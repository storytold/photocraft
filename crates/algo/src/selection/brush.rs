//! Incremental, image-independent selection strokes. Coverage is non-accumulating within
//! a stroke; document selection composition belongs to the existing selection operators.
use super::{MAX_PAINTED_TILES, combine_sparse, combine_surfaces, polygon_surface};
use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::Surface;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub size: f32,
    pub hardness: f32,
    pub opacity: f32,
    pub fill_closed: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { size: 60.0, hardness: 0.7, opacity: 1.0, fill_closed: true }
    }
}

#[derive(Clone)]
pub struct StrokeMask {
    options: Options,
    canvas: Rect,
    points: Vec<[f64; 2]>,
    length: f64,
    work: u64,
    mask: Surface,
}

impl StrokeMask {
    pub fn new(options: Options, canvas: Rect) -> Result<Self, &'static str> {
        if !options.size.is_finite()
            || options.size <= 0.0
            || options.size > 5000.0
            || !options.hardness.is_finite()
            || !(0.0..=1.0).contains(&options.hardness)
            || !options.opacity.is_finite()
            || !(0.0..=1.0).contains(&options.opacity)
        {
            return Err("invalid selection brush size, hardness or opacity");
        }
        Ok(Self { options, canvas, points: Vec::new(), length: 0.0, work: 0, mask: Surface::new(PixelFormat::GRAY8) })
    }

    pub fn mask(&self) -> &Surface {
        &self.mask
    }
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    /// Feed only new samples. The returned rectangle bounds the changed coverage; previous
    /// segments and unrelated tiles are not rasterised again.
    pub fn append(&mut self, points: &[[f64; 2]]) -> Result<Rect, &'static str> {
        if self.points.len().saturating_add(points.len()) > 65_536 || points.iter().flatten().any(|v| !v.is_finite() || v.abs() > 10_000_000.0) {
            return Err("use at most 65536 finite points within the coordinate limit");
        }
        self.points.try_reserve(points.len()).map_err(|_| "not enough memory for stroke samples")?;
        let mut dirty = Rect::EMPTY;
        for &b in points {
            let a = self.points.last().copied().unwrap_or(b);
            self.length += (b[0] - a[0]).hypot(b[1] - a[1]);
            self.points.push(b);
            if self.options.opacity == 0.0 {
                continue;
            }
            dirty = dirty.union(&self.capsule(a, b)?);
        }
        Ok(dirty)
    }

    pub fn is_closed(&self) -> bool {
        let Some((first, last)) = self.points.first().zip(self.points.last()) else { return false };
        self.options.fill_closed
            && self.points.len() >= 4
            && self.length >= f64::from(self.options.size) * 3.0
            && (first[0] - last[0]).hypot(first[1] - last[1]) <= (f64::from(self.options.size) * 0.5).max(2.0)
    }

    /// Closed interiors are added only on release, using the original image coordinates.
    /// Preview scale and viewport zoom never participate in the closure decision.
    pub fn finish(&self) -> Result<Surface, &'static str> {
        if self.points.is_empty() {
            return Err("needs at least one point");
        }
        if !self.is_closed() || self.options.opacity == 0.0 {
            return Ok(self.mask.clone());
        }
        let mut bounds = Rect::EMPTY;
        for [x, y] in &self.points {
            bounds = bounds.union(&Rect::new(x.floor() as i32, y.floor() as i32, x.ceil() as i32 + 1, y.ceil() as i32 + 1));
        }
        if bounds.intersect(&self.canvas).size().area() > 64 * 1024 * 1024 {
            return Err("closed stroke exceeds the fill work budget; use smaller closed strokes");
        }
        let points: Vec<(f32, f32)> = self.points.iter().map(|p| (p[0] as f32, p[1] as f32)).collect();
        let fill = polygon_surface(&points, self.canvas).ok_or("closed stroke exceeds the painted tile budget")?;
        let opacity = self.options.opacity;
        let fill = if opacity == 1.0 { fill } else { combine_surfaces(&fill, &fill, self.canvas, |a, _| a * opacity) };
        Ok(combine_sparse(&self.mask, &fill, self.canvas, f32::max))
    }

    fn capsule(&mut self, a: [f64; 2], b: [f64; 2]) -> Result<Rect, &'static str> {
        let radius = f64::from(self.options.size) * 0.5;
        let area = Rect::new(
            (a[0].min(b[0]) - radius - 1.0).floor() as i32,
            (a[1].min(b[1]) - radius - 1.0).floor() as i32,
            (a[0].max(b[0]) + radius + 1.0).ceil() as i32,
            (a[1].max(b[1]) + radius + 1.0).ceil() as i32,
        )
        .intersect(&self.canvas);
        if area.is_empty() {
            return Ok(Rect::EMPTY);
        }
        if area.size().area() / (TILE_SIZE as u64 * TILE_SIZE as u64) > 1_000_000 {
            return Err("stroke segment is too large");
        }
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let denom = dx * dx + dy * dy;
        let distance = |x: f64, y: f64| {
            let (px, py) = (x - a[0], y - a[1]);
            let t = if denom > 0.0 { ((px * dx + py * dy) / denom).clamp(0.0, 1.0) } else { 0.0 };
            (px - t * dx).hypot(py - t * dy)
        };
        let hard = radius * f64::from(self.options.hardness);
        let falloff = (radius - hard).max(0.0);
        for tc in area.tiles() {
            let tile_rect = tc.rect();
            let half = f64::from(TILE_SIZE) * 0.5;
            if distance(f64::from(tile_rect.x0) + half, f64::from(tile_rect.y0) + half) > radius + 1.0 + half * std::f64::consts::SQRT_2 {
                continue;
            }
            let r = area.intersect(&tile_rect);
            self.work = self.work.saturating_add(r.size().area());
            if self.work > 250_000_000 {
                return Err("stroke exceeds the work budget; use shorter strokes");
            }
            if self.mask.tile_count() >= MAX_PAINTED_TILES && self.mask.tile(tc).is_none() {
                return Err("stroke exceeds the painted tile budget");
            }
            let data = self.mask.try_tile_mut(tc).map_err(|_| "not enough memory for selection coverage")?.bytes_mut();
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    let d = distance(f64::from(x) + 0.5, f64::from(y) + 0.5);
                    let coverage =
                        if falloff <= f64::EPSILON { (radius + 0.5 - d).clamp(0.0, 1.0) } else { ((radius + 0.5 - d) / (falloff + 0.5)).clamp(0.0, 1.0) };
                    let v = (coverage * f64::from(self.options.opacity) * 255.0).round() as u8;
                    let index = (y - tile_rect.y0) as usize * TILE_SIZE as usize + (x - tile_rect.x0) as usize;
                    if let Some(old) = data.get_mut(index) {
                        *old = (*old).max(v);
                    }
                }
            }
        }
        Ok(area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mask(points: &[[f64; 2]], options: Options) -> Surface {
        let mut s = StrokeMask::new(options, Rect::new(0, 0, 500, 500)).unwrap();
        s.append(points).unwrap();
        s.finish().unwrap()
    }

    #[test]
    fn selection_brush_self_crossing_uses_even_odd_and_zero_strength_is_empty() {
        let bowtie = [[20., 20.], [100., 100.], [20., 100.], [100., 20.], [20., 20.]];
        let s = mask(&bowtie, Options { size: 2.0, hardness: 1.0, ..Default::default() });
        assert!(s.sample_channel(60, 30, 0) > 0.9);
        assert_eq!(s.sample_channel(30, 60, 0), 0.0);
        assert!(mask(&bowtie, Options { opacity: 0.0, ..Default::default() }).content_bounds().is_empty());
    }
    #[test]
    fn selection_brush_soft_opacity_and_non_accumulating_overlap() {
        let options = Options { size: 40.0, hardness: 0.0, opacity: 0.5, ..Default::default() };
        let s = mask(&[[100., 100.], [100., 100.], [100., 100.]], options);
        let center = s.sample_channel(100, 100, 0);
        let edge = s.sample_channel(115, 100, 0);
        assert!((center - 0.5).abs() < 0.03 && edge > 0.0 && edge < center);
        let hard = mask(&[[100., 100.]], Options { hardness: 1.0, ..options });
        assert!(hard.sample_channel(115, 100, 0) > edge);
    }
    #[test]
    fn selection_brush_incremental_matches_batch_and_only_changes_new_segment() {
        let options = Options { size: 10.0, hardness: 1.0, ..Default::default() };
        let points = [[20., 20.], [100., 20.], [100., 100.]];
        let batch = mask(&points, options);
        let mut s = StrokeMask::new(options, Rect::new(0, 0, 500, 500)).unwrap();
        s.append(&points[..2]).unwrap();
        let dirty = s.append(&points[2..]).unwrap();
        assert!(dirty.x0 > 90);
        assert_eq!(batch.read_region(Rect::new(0, 0, 150, 150)), s.finish().unwrap().read_region(Rect::new(0, 0, 150, 150)));
        assert_eq!(batch.sample_channel(50, 50, 0), 0.0);
    }
    #[test]
    fn selection_brush_closed_fill_respects_opacity_and_open_c_stays_open() {
        let options = Options { size: 10.0, opacity: 0.5, ..Default::default() };
        let open = [[20., 20.], [100., 20.], [100., 100.], [20., 100.]];
        assert_eq!(mask(&open, options).sample_channel(60, 60, 0), 0.0);
        let closed = [[20., 20.], [100., 20.], [100., 100.], [20., 100.], [20., 20.]];
        assert!((mask(&closed, options).sample_channel(60, 60, 0) - 0.5).abs() < 0.01);
        assert_eq!(mask(&closed, Options { opacity: 0.0, ..options }).tile_count(), 0);
    }
    #[test]
    fn selection_brush_huge_diagonal_is_sparse_and_bad_numbers_are_rejected() {
        let mut s = StrokeMask::new(Options { size: 2.0, ..Default::default() }, Rect::new(0, 0, 50_000, 50_000)).unwrap();
        s.append(&[[0., 0.], [49_999., 49_999.]]).unwrap();
        assert!(s.mask().tile_count() < 1000);
        assert!(s.append(&[[f64::NAN, 0.]]).is_err());
        assert!(StrokeMask::new(Options { opacity: f32::INFINITY, ..Default::default() }, Rect::EMPTY).is_err());
    }
}
