//! Maps shared source tiles to a private native surface. The importer publishes the
//! surface only after the complete drawable is decoded.

use craft_xcf::samples::{SourceSample, SourceTile};
use craft_xcf::{Error, Result};
use photocraft_color::{PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, stored_tiles_bytes, tile_storage_bytes};

pub(super) struct TileWriter<'a> {
    surface: Surface,
    origin: (i32, i32),
    palette: &'a [[u8; 3]],
    opaque: bool,
    coverage: bool,
    limit: usize,
    samples: Vec<f32>,
    f32_words: Vec<[u8; 4]>,
    admitted: bool,
    clamped: bool,
}

impl<'a> TileWriter<'a> {
    pub(super) fn new(format: PixelFormat, origin: (i32, i32), palette: &'a [[u8; 3]], opaque: bool, coverage: bool, limit: usize) -> Self {
        Self {
            surface: Surface::new(format),
            origin,
            palette,
            opaque,
            coverage,
            limit,
            samples: Vec::new(),
            f32_words: Vec::new(),
            admitted: false,
            clamped: false,
        }
    }

    pub(super) fn write(&mut self, tile: SourceTile<'_>) -> Result<()> {
        let source = tile.drawable;
        let channels = self.surface.channels();
        let expected = source.channels + 2 * usize::from(source.indexed) + usize::from(self.opaque);
        if expected != channels {
            return Err(Error::Invalid("source samples do not match the native drawable".into()));
        }
        let count = tile.width as usize * tile.height as usize * channels;
        let f32_values = if !self.opaque && !source.indexed && self.surface.format().sample == SampleType::F32 { tile.f32_samples()? } else { None };
        if !self.admitted {
            let rect = region(self.origin, 0, 0, source.width, source.height)?;
            let logical = u64::from(source.width) * u64::from(source.height) * channels as u64 * 4;
            let native = tile_storage_bytes(rect, &self.surface.format());
            let scratch = (count as u64) * 4 + tile.data.len() as u64;
            // Compare the logical size to the original balance. Check the actual
            // native allocation and the concurrently live tile scratch separately.
            if logical > self.limit as u64 || native.saturating_add(scratch) > self.limit as u64 {
                return Err(Error::Invalid("the layers exceed the 1 GiB pixel limit".into()));
            }
            let reservation = if f32_values.is_some() { self.f32_words.try_reserve_exact(count) } else { self.samples.try_reserve_exact(count) };
            reservation.map_err(|_| Error::Invalid("not enough memory for XCF tile samples".into()))?;
            self.admitted = true;
        }
        let rect = region(self.origin, tile.x, tile.y, tile.width, tile.height)?;
        if let Some(values) = f32_values {
            return self.write_f32(rect, values, source.channels);
        }
        self.samples.clear();
        let pixels = tile.width as usize * tile.height as usize;
        for pixel in 0..pixels {
            let offset = pixel * source.channels;
            let mut start = 0;
            if source.indexed {
                let SourceSample::U8(index) = tile.sample(offset)? else {
                    return Err(Error::Invalid("an index must be an eight-bit sample".into()));
                };
                let rgb = self.palette.get(usize::from(index)).copied().unwrap_or([index; 3]);
                self.samples.extend(rgb.into_iter().map(|value| f32::from(value) / 255.0));
                start = 1;
            }
            for component in start..source.channels {
                let source_value = tile.sample(offset + component)?.normalized_f64();
                if !source_value.is_finite() {
                    return Err(Error::Invalid("a source sample is not finite".into()));
                }
                let is_coverage = self.coverage || (matches!(source.channels, 2 | 4) && component + 1 == source.channels);
                let value = if is_coverage { source_value.clamp(0.0, 1.0) } else { source_value };
                let sample = value as f32;
                if !sample.is_finite() {
                    return Err(Error::Invalid("a source sample cannot be represented in the native drawable".into()));
                }
                self.samples.push(sample);
                self.clamped |= value != source_value;
            }
            if self.opaque {
                self.samples.push(1.0);
            }
        }
        self.surface.try_write_region(rect, &self.samples).map_err(|error| Error::Invalid(error.to_string()))
    }

    fn write_f32(&mut self, rect: Rect, values: impl ExactSizeIterator<Item = f32>, channels: usize) -> Result<()> {
        self.f32_words.clear();
        let mut nonfinite = false;
        self.f32_words.extend(values.map(|value| {
            nonfinite |= !value.is_finite();
            value.to_le_bytes()
        }));
        if nonfinite {
            return Err(Error::Invalid("a source sample is not finite".into()));
        }
        // The native F32 surface uses little-endian words. Preserve HDR color
        // as stored and bound only the coverage samples before writing.
        if self.coverage {
            self.clamped |= clamp_f32_coverage(self.f32_words.iter_mut());
        } else if matches!(channels, 2 | 4) {
            self.clamped |= clamp_f32_coverage(self.f32_words.chunks_exact_mut(channels).filter_map(<[_]>::last_mut));
        }
        self.surface.try_write_interleaved(rect, self.f32_words.as_flattened()).map_err(|error| Error::Invalid(error.to_string()))
    }

    pub(super) fn finish(mut self, budget: &mut usize) -> (Surface, bool) {
        self.surface.prune();
        let bytes = stored_tiles_bytes(self.surface.tile_count() as u64, &self.surface.format());
        *budget = budget.saturating_sub(usize::try_from(bytes).unwrap_or(usize::MAX));
        (self.surface, self.clamped)
    }
}

fn clamp_f32_coverage<'a>(words: impl Iterator<Item = &'a mut [u8; 4]>) -> bool {
    let mut clamped = false;
    for word in words {
        let original = f32::from_le_bytes(*word);
        let value = original.clamp(0.0, 1.0);
        if value != original {
            *word = value.to_le_bytes();
            clamped = true;
        }
    }
    clamped
}

fn region(origin: (i32, i32), x: u32, y: u32, width: u32, height: u32) -> Result<Rect> {
    let coordinate = |base: i32, delta: u32| {
        i32::try_from(delta).ok().and_then(|delta| base.checked_add(delta)).ok_or_else(|| Error::Invalid("drawable offset overflow".into()))
    };
    let x = coordinate(origin.0, x)?;
    let y = coordinate(origin.1, y)?;
    Ok(Rect::new(x, y, coordinate(x, width)?, coordinate(y, height)?))
}
