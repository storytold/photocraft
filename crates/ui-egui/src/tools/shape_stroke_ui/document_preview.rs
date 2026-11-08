//! Bounded box reduction of committed document pixels for large drag previews.

use photocraft_geom::Rect;
use photocraft_vector::CompiledShape;

pub(super) const TEXTURE_EDGE: u32 = 2048;
const TILE_EDGE: u32 = 256;
pub(super) const PIXELS_PER_PASS: u64 = 4_194_304;

/// Integer document-pixel footprints keep native coverage and a bounded texture allocation.
#[derive(Clone, Copy)]
pub(super) struct PreviewLayout {
    pub bounds: Rect,
    pub size: [usize; 2],
    pub factor: u32,
}

impl PreviewLayout {
    /// Select an integer reduction factor with at most 2048 texels per dimension.
    pub fn new(bounds: Rect) -> Option<Self> {
        if bounds.is_empty() {
            return None;
        }
        let factor = bounds.width().max(bounds.height()).div_ceil(TEXTURE_EDGE).max(1);
        let size = [bounds.width().div_ceil(factor) as usize, bounds.height().div_ceil(factor) as usize];
        // Even i32-extreme document bounds cannot create an oversized allocation.
        (size[0] <= TEXTURE_EDGE as usize && size[1] <= TEXTURE_EDGE as usize && size[0].checked_mul(size[1])? <= PIXELS_PER_PASS as usize).then_some(Self {
            bounds,
            size,
            factor,
        })
    }
}

/// Stream small native tiles into premultiplied sums, never rasterizing the entire source box.
pub(super) struct PreviewReduction {
    compiled: CompiledShape,
    layout: PreviewLayout,
    sums: Vec<[f64; 4]>,
    cursor: [i32; 2],
}

impl PreviewReduction {
    /// Allocate only the validated output grid, retaining the compiled document renderer.
    pub fn new(compiled: CompiledShape, layout: PreviewLayout) -> Self {
        Self { compiled, layout, sums: vec![[0.0; 4]; layout.size[0] * layout.size[1]], cursor: [layout.bounds.x0, layout.bounds.y0] }
    }

    /// Every native source tile has contributed to the reduced image.
    pub fn complete(&self) -> bool {
        self.cursor[1] >= self.layout.bounds.y1
    }

    /// Cap work as well as memory so hostile/extreme document bounds cannot freeze a frame.
    /// The caller requests another frame while there are unprocessed tiles.
    pub fn advance(&mut self) -> u64 {
        let mut used = 0;
        while !self.complete() {
            let tile = Rect::new(
                self.cursor[0],
                self.cursor[1],
                self.cursor[0].saturating_add(TILE_EDGE as i32).min(self.layout.bounds.x1),
                self.cursor[1].saturating_add(TILE_EDGE as i32).min(self.layout.bounds.y1),
            );
            let count = u64::from(tile.width()) * u64::from(tile.height());
            if count == 0 || count > PIXELS_PER_PASS - used {
                break;
            }
            let pixels = self.compiled.render_rgba(tile);
            self.accumulate(tile, &pixels);
            used += count;
            self.cursor[0] = tile.x1;
            if self.cursor[0] >= self.layout.bounds.x1 {
                self.cursor = [self.layout.bounds.x0, tile.y1];
            }
        }
        used
    }

    /// Box filtering must average premultiplied colour, including transparent edge pixels.
    fn accumulate(&mut self, tile: Rect, pixels: &[[f32; 4]]) {
        let factor = self.layout.factor as usize;
        for (i, pixel) in pixels.iter().enumerate() {
            let x = i64::from(tile.x0) - i64::from(self.layout.bounds.x0) + (i % tile.width() as usize) as i64;
            let y = i64::from(tile.y0) - i64::from(self.layout.bounds.y0) + (i / tile.width() as usize) as i64;
            let index = (y as usize / factor) * self.layout.size[0] + x as usize / factor;
            let Some(sum) = self.sums.get_mut(index) else { continue };
            let alpha = f64::from(pixel[3].clamp(0.0, 1.0));
            for channel in 0..3 {
                sum[channel] += f64::from(pixel[channel].clamp(0.0, 1.0)) * alpha;
            }
            sum[3] += alpha;
        }
    }

    /// Unvisited source pixels and partial final footprints are transparent, not stretched.
    pub fn image(&self) -> egui::ColorImage {
        let area = f64::from(self.layout.factor).powi(2);
        let pixels = self
            .sums
            .iter()
            .map(|sum| {
                if sum[3] <= 0.0 {
                    return egui::Color32::TRANSPARENT;
                }
                let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                egui::Color32::from_rgba_unmultiplied(byte(sum[0] / sum[3]), byte(sum[1] / sum[3]), byte(sum[2] / sum[3]), byte(sum[3] / area))
            })
            .collect();
        egui::ColorImage::new(self.layout.size, pixels)
    }
}
