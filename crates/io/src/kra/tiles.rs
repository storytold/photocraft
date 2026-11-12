//! Krita tile files (parsed by craft-kra) onto surfaces: pixel layouts to surface formats, the
//! memory a file's tiles take once placed, and the tiles written into a surface.

use craft_kra::tiles::{Space, TileFile};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

use super::{Result, invalid, kra_err};
use crate::IoError;

/// What a Krita pixel layout is in this app's terms.
pub(crate) trait SpaceExt {
    /// The document colour mode and depth for an image in this space.
    fn model(self) -> (ColorMode, SampleType);
    /// The surface format whose bytes [`Space::samples`] writes directly.
    fn native(self) -> PixelFormat;
    /// Straight RGBA samples as `fmt`'s channels.
    fn to_format(self, rgba: &[f32; 4], fmt: PixelFormat) -> Vec<f32>;
    /// Interleaved Krita pixels as encoded `fmt` bytes.
    fn convert(self, src: &[u8], fmt: PixelFormat, out: &mut Vec<u8>);
}

impl SpaceExt for Space {
    fn model(self) -> (ColorMode, SampleType) {
        match self {
            Space::Rgba8 => (ColorMode::Rgb, SampleType::U8),
            Space::Rgba16 => (ColorMode::Rgb, SampleType::U16),
            Space::Gray8 | Space::Alpha8 => (ColorMode::Grayscale, SampleType::U8),
            Space::Gray16 => (ColorMode::Grayscale, SampleType::U16),
        }
    }
    fn native(self) -> PixelFormat {
        match self {
            Space::Alpha8 => PixelFormat::GRAY8,
            s => {
                let (mode, depth) = s.model();
                PixelFormat::new(mode, depth, true)
            }
        }
    }
    fn to_format(self, rgba: &[f32; 4], fmt: PixelFormat) -> Vec<f32> {
        photocraft_raster::from_rgba(&fmt, *rgba)
    }
    fn convert(self, src: &[u8], fmt: PixelFormat, out: &mut Vec<u8>) {
        if fmt == self.native() {
            self.samples(src, out);
            return;
        }
        out.clear();
        let mut enc = vec![0; fmt.bytes_per_pixel()];
        for px in src.chunks_exact(self.pixel_size()) {
            let v = photocraft_raster::from_rgba(&fmt, self.rgba(px));
            photocraft_raster::encode_pixel(&fmt, &v, &mut enc);
            out.extend_from_slice(&enc);
        }
    }
}

/// Bytes the decoded tiles take at `bpp` bytes per pixel: the larger of their own pixels and
/// the whole 256×256 backing tiles they land in (a surface stores no less than a tile, so
/// sparse tiles at distinct positions retain far more than they hold).
pub(crate) fn decoded_bytes(file: &TileFile<'_>, bpp: usize, dx: i32, dy: i32) -> Result<usize> {
    tile_cost(file.tiles().iter().map(|t| (t.x, t.y)), file.width, file.height, bpp, dx, dy)
}

fn tile_cost(positions: impl ExactSizeIterator<Item = (i32, i32)>, width: usize, height: usize, bpp: usize, dx: i32, dy: i32) -> Result<usize> {
    let overflow = || invalid("tile size overflow");
    let logical = positions.len().checked_mul(width * height).and_then(|n| n.checked_mul(bpp)).ok_or_else(overflow)?;
    let side = photocraft_geom::TILE_SIZE;
    let (w, h) = (i32::try_from(width.max(1)).map_err(|_| overflow())?, i32::try_from(height.max(1)).map_err(|_| overflow())?);
    let mut backing = std::collections::HashSet::new();
    for (x, y) in positions {
        // The tile lands at its offset position and covers every backing tile its box overlaps.
        let (x, y) = (x.checked_add(dx).ok_or_else(overflow)?, y.checked_add(dy).ok_or_else(overflow)?);
        let (x1, y1) = (x.checked_add(w - 1).ok_or_else(overflow)?, y.checked_add(h - 1).ok_or_else(overflow)?);
        for c in x.div_euclid(side)..=x1.div_euclid(side) {
            for r in y.div_euclid(side)..=y1.div_euclid(side) {
                backing.insert((c, r));
            }
        }
    }
    let tile_bytes = (side as usize).checked_mul(side as usize).and_then(|n| n.checked_mul(bpp)).ok_or_else(overflow)?;
    Ok(logical.max(backing.len().checked_mul(tile_bytes).ok_or_else(overflow)?))
}

/// Writes every tile into `surface` (format `surface.format()`), offset by (`dx`, `dy`).
pub(crate) fn decode_into(file: &TileFile<'_>, surface: &mut Surface, space: Space, dx: i32, dy: i32, ctl: &Interrupt<'_>) -> Result<()> {
    let fmt = surface.format();
    let n = file.width * file.height;
    let mut interleaved = Vec::new();
    let mut out = Vec::new();
    for tile in file.tiles() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        file.decode(tile, &mut interleaved).map_err(kra_err)?;
        space.convert(&interleaved, fmt, &mut out);
        let (x0, y0) =
            (tile.x.checked_add(dx).ok_or_else(|| invalid("tile offset overflow"))?, tile.y.checked_add(dy).ok_or_else(|| invalid("tile offset overflow"))?);
        let rect = Rect::from_xywh(x0, y0, file.width as u32, file.height as u32);
        if rect.width() as usize != file.width || rect.height() as usize != file.height || out.len() != n * fmt.bytes_per_pixel() {
            return Err(invalid("tile does not fit its position"));
        }
        surface.write_interleaved(rect, &out);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(positions: &[(i32, i32)], width: usize, dx: i32, dy: i32) -> usize {
        tile_cost(positions.iter().copied(), width, if width == 64 { 64 } else { 1 }, 4, dx, dy).unwrap()
    }

    #[test]
    fn sparse_tiles_are_charged_for_the_backing_tiles_they_land_in() {
        // Sixteen 64×64 tiles that fill one 256×256 backing tile cost what they hold.
        let dense: Vec<_> = (0..4).flat_map(|y| (0..4).map(move |x| (x * 64, y * 64))).collect();
        assert_eq!(cost(&dense, 64, 0, 0), 16 * 64 * 64 * 4);
        // The same number far apart each take a whole backing tile: 16 × 256 × 256 × 4.
        let sparse: Vec<_> = (0..16).map(|i| (i * 10_240, 0)).collect();
        assert_eq!(cost(&sparse, 64, 0, 0), 16 * 256 * 256 * 4);
    }

    #[test]
    fn the_placement_offset_and_tile_extent_decide_the_backing_tiles() {
        // One 64×64 tile straddling a backing-tile corner once offset touches four.
        assert_eq!(cost(&[(0, 0)], 64, 224, 224), 4 * 256 * 256 * 4);
        // A tile wider than a backing tile reaches the ones between its corners.
        assert_eq!(cost(&[(0, 0)], 1024, 0, 0), 4 * 256 * 256 * 4);
    }
}
