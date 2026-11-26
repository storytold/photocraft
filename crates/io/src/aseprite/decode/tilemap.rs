//! Bounded embedded tilesets and 32-bit tile maps. Original records are retained separately
//! from the raster display: editing another layer must never replace a tile map with pixels.

use super::{ColorMode, Decoder, IoError, PixelFormat, Reader, Rect, Result, SampleType, Surface, invalid};
use photocraft_doc::sprite::{self, MetadataTarget, SpriteChunk};
use photocraft_raster::Interrupt;
use std::{io::Read, sync::Arc};

pub(in crate::aseprite) struct Tileset {
    pub id: u32,
    pub width: u16,
    pub height: u16,
    pub count: u32,
    pub flags: u32,
    pub pixels: Option<Vec<u8>>,
}

/// Inflate exactly the declared byte count, checking cancellation, checksum and trailing input.
pub(in crate::aseprite) fn inflate(data: &[u8], expected: usize, ctl: &Interrupt<'_>) -> Result<Vec<u8>> {
    if expected > sprite::MAX_BYTES {
        return Err(invalid("tile data exceeds 256 MiB"));
    }
    let mut z = flate2::bufread::ZlibDecoder::new(data);
    let mut pixels = Vec::new();
    let mut limited = (&mut z).take(expected as u64 + 1);
    let mut buffer = vec![0; 64 * 1024];
    loop {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let n = limited.read(&mut buffer).map_err(|e| invalid(format!("invalid tile data: {e}")))?;
        if n == 0 {
            break;
        }
        append_bounded(&mut pixels, buffer.get(..n).ok_or_else(|| invalid("invalid inflated tile block size"))?, expected)?;
    }
    if pixels.len() != expected || z.total_in() != data.len() as u64 {
        return Err(invalid("tile data length/checksum/trailing bytes are invalid"));
    }
    Ok(pixels)
}

/// Extend decoded tile storage only after checked size and budget validation.
fn append_bounded(pixels: &mut Vec<u8>, block: &[u8], expected: usize) -> Result<()> {
    let size = pixels.len().checked_add(block.len()).filter(|n| *n <= expected).ok_or_else(|| invalid("inflated tile data exceeds its declared size"))?;
    if size > pixels.capacity() {
        // Don't double past the declared budget, including on the extra byte used to detect
        // an oversized stream. Allocation failure is an import error, not an abort.
        let capacity = pixels.capacity().saturating_mul(2).max(64 * 1024).max(size).min(expected);
        pixels.try_reserve_exact(capacity.saturating_sub(pixels.len())).map_err(|e| invalid(format!("cannot allocate tile data: {e}")))?;
    }
    pixels.extend_from_slice(block);
    Ok(())
}

/// Parse embedded/external tileset descriptors and enforce the remaining decoded-image budget first.
pub(in crate::aseprite) fn tileset(data: &[u8], bpp: usize, budget: usize, ctl: &Interrupt<'_>) -> Result<Tileset> {
    let mut r = Reader::new(data);
    let id = r.u32()?;
    let flags = r.u32()?;
    let count = r.u32()?;
    let (width, height) = (r.u16()?, r.u16()?);
    r.take(16)?;
    r.string()?;
    if flags & !63 != 0 || count == 0 || width == 0 || height == 0 {
        return Err(invalid("invalid tileset flags/count/dimensions"));
    }
    if flags & 1 != 0 {
        r.take(8)?;
    }
    let expected = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|n| n.checked_mul(count as usize))
        .and_then(|n| n.checked_mul(bpp))
        .ok_or_else(|| invalid("tileset size overflow"))?;
    if expected > sprite::MAX_BYTES {
        return Err(invalid("tileset image exceeds 256 MiB"));
    }
    let pixels = if flags & 2 != 0 {
        if expected > budget {
            return Err(invalid("tilesets exceed the remaining decoded memory budget"));
        }
        let len = r.u32()? as usize;
        Some(inflate(r.take(len)?, expected, ctl)?)
    } else {
        None
    };
    r.end()?;
    Ok(Tileset { id, width, height, count, flags, pixels })
}

/// The stable tileset index named by the layer and the tileset record, not its order of
/// appearance. External-file ids are a different namespace.
pub(in crate::aseprite) fn layer_tileset(data: &[u8]) -> Result<usize> {
    let mut r = Reader::new(data);
    r.u16()?;
    if r.u16()? != 2 {
        return Err(invalid("retained tilemap layer has the wrong type"));
    }
    r.take(12)?;
    r.string()?;
    let index = r.u32()? as usize;
    if !matches!(r.remaining(), 0 | 16) {
        return Err(invalid("invalid tilemap layer record"));
    }
    Ok(index)
}

pub(in crate::aseprite) struct Map<'a> {
    width: u16,
    height: u16,
    masks: [u32; 4],
    compressed: &'a [u8],
}

impl Map<'_> {
    pub(in crate::aseprite) fn rect(&self, tileset: &Tileset, at: (i32, i32), bpp: usize) -> Result<Rect> {
        let width = u32::from(self.width) * u32::from(tileset.width);
        let height = u32::from(self.height) * u32::from(tileset.height);
        let rect = Rect::from_xywh(at.0, at.1, width, height);
        if rect.width() != width || rect.height() != height {
            return Err(invalid("tilemap rectangle exceeds the supported coordinate range"));
        }
        sprite::pixels::checked_rect(rect, bpp).map_err(invalid)?;
        Ok(rect)
    }

    pub(in crate::aseprite) fn inflate(&self, budget: usize, ctl: &Interrupt<'_>) -> Result<Vec<u8>> {
        let expected =
            usize::from(self.width).checked_mul(usize::from(self.height)).and_then(|n| n.checked_mul(4)).ok_or_else(|| invalid("tile map size overflow"))?;
        if expected > budget {
            return Err(invalid("tile map words exceed the remaining decoded memory budget"));
        }
        inflate(self.compressed, expected, ctl)
    }
}

/// Parse dimensions and transformation masks before allocating map words.
pub(in crate::aseprite) fn map(data: &[u8]) -> Result<Map<'_>> {
    let mut r = Reader::new(data);
    r.take(7)?;
    if r.u16()? != 3 {
        return Err(invalid("retained tilemap cel has the wrong type"));
    }
    r.take(7)?;
    let (width, height, bits) = (r.u16()?, r.u16()?, r.u16()?);
    if bits != 32 {
        return Err(invalid("only documented 32-bit tile ids are supported"));
    }
    let masks = [r.u32()?, r.u32()?, r.u32()?, r.u32()?];
    r.take(10)?;
    let id = *masks.first().ok_or_else(|| invalid("tile id mask is missing"))?;
    if id == 0 || !((id >> id.trailing_zeros()) as u64 + 1).is_power_of_two() {
        return Err(invalid("invalid tile id mask"));
    }
    let mut used = id;
    for m in masks.iter().skip(1) {
        if *m != 0 && (!m.is_power_of_two() || used & m != 0) {
            return Err(invalid("overlapping or invalid tile transform masks"));
        }
        used |= m;
    }
    let compressed = r.take(r.remaining())?;
    Ok(Map { width, height, masks, compressed })
}

/// Reserve the surface's full tile footprint before allocating any transient map words.
pub(in crate::aseprite) fn after_display(rect: Rect, bpp: usize, budget: usize) -> Result<usize> {
    let bytes = usize::try_from(rect.tile_count()).ok().and_then(|n| n.checked_mul(256 * 256 * bpp)).ok_or_else(|| invalid("cel tile size overflow"))?;
    budget.checked_sub(bytes).ok_or_else(|| invalid("decoded sprite tiles exceed the remaining memory budget (256 MiB total)"))
}

/// Render supported tile transformations while refusing invalid IDs and non-square diagonal tiles.
pub(in crate::aseprite) fn render(
    map: &Map<'_>,
    tiles: &[u8],
    tileset: &Tileset,
    rect: Rect,
    fmt: PixelFormat,
    transparent: Option<u8>,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    let data =
        tileset.pixels.as_ref().ok_or_else(|| invalid("tilemap needs an external-only tileset; embed it in Aseprite first (no external files were read)"))?;
    let (tw, th) = (u32::from(tileset.width), u32::from(tileset.height));
    let mut out = if let Some(index) = transparent { Surface::with_default(fmt, &[f32::from(index) / 255.0]) } else { Surface::new(fmt) };
    let bpp = fmt.bytes_per_pixel();
    if rect.is_empty() {
        return Ok(out);
    }
    let [id_mask, x_mask, y_mask, d_mask] = map.masks;
    for (i, word) in tiles.as_chunks::<4>().0.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let value = u32::from_le_bytes(*word);
        let id = (value & id_mask) >> id_mask.trailing_zeros();
        if tileset.flags & 4 != 0 && id == 0 || tileset.flags & 4 == 0 && value == u32::MAX {
            continue;
        }
        if id >= tileset.count {
            return Err(invalid("tile id exceeds the tileset image"));
        }
        let (flip_x, flip_y, diagonal) = (value & x_mask != 0, value & y_mask != 0, value & d_mask != 0);
        if diagonal && tw != th {
            return Err(invalid("diagonal transforms require square tiles"));
        }
        let (cx, cy) = (i % usize::from(map.width), i / usize::from(map.width));
        for dy in 0..th {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let mut row = Vec::with_capacity(tw as usize * bpp);
            for dx in 0..tw {
                let (mut sx, mut sy) = (dx, dy);
                if flip_x {
                    sx = tw - 1 - sx
                }
                if flip_y {
                    sy = th - 1 - sy
                }
                if diagonal {
                    std::mem::swap(&mut sx, &mut sy)
                }
                let offset = (((id as usize * th as usize + sy as usize) * tw as usize) + sx as usize)
                    .checked_mul(bpp)
                    .ok_or_else(|| invalid("tile pixel offset overflow"))?;
                row.extend_from_slice(data.get(offset..offset + bpp).ok_or_else(|| invalid("tileset pixel is missing"))?);
            }
            let x = rect.x0 + cx as i32 * tw as i32;
            let y = rect.y0 + cy as i32 * th as i32 + dy as i32;
            out.try_write_interleaved(Rect::from_xywh(x, y, tw, 1), &row).map_err(|e| invalid(e.to_string()))?;
        }
    }
    Ok(out)
}

impl Decoder<'_> {
    /// Retain tile records and their object targets within the aggregate metadata limit.
    pub(super) fn retain_tile_record(&mut self, kind: u16, data: &[u8], target: MetadataTarget) -> Result<()> {
        self.metadata_bytes = self.metadata_bytes.checked_add(data.len()).ok_or_else(|| invalid("tile metadata size overflow"))?;
        if self.metadata_bytes > sprite::MAX_METADATA {
            return Err(invalid("retained tile records exceed 32 MiB"));
        }
        self.frame.metadata.push(SpriteChunk { kind, target, bytes: Arc::new(data.to_vec()) });
        Ok(())
    }
    /// Preflight combined tileset storage before inflation, then build locked frame display caches.
    pub(super) fn render_tilemaps(&mut self) -> Result<()> {
        let mut sets = std::collections::BTreeMap::new();
        let source_bpp = if self.indexed { 1 } else { self.fmt.bytes_per_pixel() };
        let mut decoded = 0usize;
        for frame in &self.frames {
            for c in &frame.metadata {
                if c.kind == 0x2023 {
                    self.ctl.check().map_err(|_| IoError::Cancelled)?;
                    let set = tileset(&c.bytes, source_bpp, self.budget.saturating_sub(decoded), self.ctl)?;
                    decoded = decoded.checked_add(set.pixels.as_ref().map_or(0, Vec::len)).ok_or_else(|| invalid("tileset size overflow"))?;
                    if decoded > self.budget {
                        return Err(invalid("tilesets exceed the remaining decoded memory budget"));
                    }
                    if sets.insert(set.id as usize, set).is_some() {
                        return Err(invalid("duplicate tileset index"));
                    }
                }
            }
        }
        self.budget = self.budget.checked_sub(decoded).ok_or_else(|| invalid("tileset budget exceeded"))?;
        let mut layer_sets = std::collections::BTreeMap::new();
        for descriptor in self.frames.first().into_iter().flat_map(|frame| &frame.metadata).filter(|c| c.kind == 0x2004) {
            let MetadataTarget::Layer(id) = descriptor.target else { return Err(invalid("tilemap layer attachment is invalid")) };
            let set = sets.get(&layer_tileset(&descriptor.bytes)?).ok_or_else(|| invalid("tilemap layer refers to a missing tileset"))?;
            layer_sets.insert(id, set);
        }
        for frame_index in 0..self.frames.len() {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            let records = self.frames.get(frame_index).map(|f| f.metadata.iter().filter(|c| c.kind == 0x2005).cloned().collect::<Vec<_>>()).unwrap_or_default();
            for record in records {
                let MetadataTarget::Cel(id) = record.target else { return Err(invalid("tile cel attachment is invalid")) };
                let set = layer_sets.get(&id).ok_or_else(|| invalid("tile cel has no tilemap layer"))?;
                let parsed = map(&record.bytes)?;
                let frame = self.frames.get(frame_index).ok_or_else(|| invalid("tilemap frame is missing"))?;
                let cel = frame.cels.get(&id).ok_or_else(|| invalid("tilemap cel is missing"))?.clone();
                let palette = frame.palette.clone();
                let bpp = if self.indexed { 5 } else { self.fmt.bytes_per_pixel() };
                let rect = parsed.rect(set, (cel.rect.x0, cel.rect.y0), bpp)?;
                let fmt = if self.indexed { PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false) } else { self.fmt };
                // Map words are dropped before rendering the RGB cache of an indexed display.
                let remaining = after_display(rect, fmt.bytes_per_pixel(), self.budget)?;
                self.charge(rect, bpp)?;
                let tiles = parsed.inflate(remaining, self.ctl)?;
                let pixels = render(&parsed, &tiles, set, rect, fmt, self.indexed.then_some(self.transparent), self.ctl)?;
                drop(tiles);
                let display = if self.indexed {
                    sprite::pixels::render_indices(&pixels, rect, &palette, Some(self.transparent)).map_err(invalid)?
                } else {
                    pixels.clone()
                };
                let mut replacement = cel.as_ref().clone();
                replacement.rect = rect;
                replacement.pixels = display;
                replacement.indices = self.indexed.then_some(pixels);
                Arc::make_mut(self.frames.get_mut(frame_index).ok_or_else(|| invalid("tilemap frame is missing"))?).cels.insert(id, Arc::new(replacement));
            }
        }
        drop(sets);
        self.budget += decoded;
        Ok(())
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::{
        io::Write,
        sync::atomic::{AtomicUsize, Ordering},
    };

    /// Build synthetic tileset records for exact-boundary allocation tests.
    fn record(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for n in [0u32, 2, 4] {
            out.extend(n.to_le_bytes());
        }
        out.extend(1u16.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend([0; 18]); // base index/reserved and empty name
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    /// Budget refusal must precede malformed compressed data, while an exact-sized tileset still decodes.
    #[test]
    fn tileset_budget_is_checked_before_inflation_and_accepts_the_exact_remaining_size() {
        let checks = AtomicUsize::new(0);
        let cancel = || {
            checks.fetch_add(1, Ordering::Relaxed);
            false
        };
        let malformed = record(b"not zlib");
        assert!(tileset(&malformed, 4, 15, &Interrupt::cancel_only(&cancel)).err().unwrap().to_string().contains("remaining decoded memory budget"));
        assert_eq!(checks.load(Ordering::Relaxed), 0, "do not begin inflating an over-budget record");
        assert!(tileset(&malformed, 4, 16, &Interrupt::NONE).is_err());
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
        encoder.write_all(&[7; 16]).unwrap();
        let valid = record(&encoder.finish().unwrap());
        assert_eq!(tileset(&valid, 4, 16, &Interrupt::NONE).unwrap().pixels.unwrap(), [7; 16]);
        // A preceding tileset consumes one byte of the caller's total allowance.
        assert!(tileset(&valid, 4, 16usize.saturating_sub(1), &Interrupt::NONE).is_err());
    }

    #[test]
    fn oversized_output_is_refused_before_growing_or_changing_the_buffer() {
        let mut pixels = vec![7; 4];
        let capacity = pixels.capacity();
        assert!(append_bounded(&mut pixels, &[8], 4).is_err());
        assert_eq!(pixels, [7; 4]);
        assert_eq!(pixels.capacity(), capacity);
        append_bounded(&mut pixels, &[8], 5).unwrap();
        assert_eq!(pixels, [7, 7, 7, 7, 8]);
        assert_eq!(pixels.capacity(), 5, "successful growth is capped at the declared length");
    }

    /// Cancellation remains responsive without accepting bad checksums or trailing compressed bytes.
    #[test]
    fn inflation_checks_cancellation_between_blocks_without_weakening_zlib_validation() {
        let raw = vec![7; 1024 * 1024];
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
        encoder.write_all(&raw).unwrap();
        let compressed = encoder.finish().unwrap();
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 2;
        assert!(matches!(inflate(&compressed, raw.len(), &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(checks.load(Ordering::Relaxed), 3, "cancel during inflation rather than after the whole tileset");
        assert_eq!(inflate(&compressed, raw.len(), &Interrupt::NONE).unwrap(), raw);
        assert!(inflate(&compressed, raw.len() - 1, &Interrupt::NONE).is_err());
        assert!(inflate(&compressed, raw.len() + 1, &Interrupt::NONE).is_err());
        assert!(inflate(&compressed[..compressed.len() - 1], raw.len(), &Interrupt::NONE).is_err());
        let mut trailing = compressed;
        trailing.push(0);
        assert!(inflate(&trailing, raw.len(), &Interrupt::NONE).is_err());
    }

    #[test]
    fn map_words_respect_the_remaining_budget_and_render_can_cancel() {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
        encoder.write_all(&[0; 16]).unwrap();
        let compressed = encoder.finish().unwrap();
        let map = Map { width: 2, height: 2, masks: [1, 0, 0, 0], compressed: &compressed };
        let set = Tileset { id: 0, width: 1, height: 1, count: 1, flags: 0, pixels: Some(vec![255; 4]) };

        assert!(map.rect(&set, (i32::MAX, 0), 4).is_err());
        let rect = map.rect(&set, (0, 0), 4).unwrap();
        let error = map.inflate(15, &Interrupt::NONE).unwrap_err().to_string();
        assert!(error.contains("map words exceed"), "{error}");
        let tiles = map.inflate(16, &Interrupt::NONE).unwrap();
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) == 2;
        assert!(matches!(render(&map, &tiles, &set, rect, PixelFormat::RGBA8, None, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(render(&map, &tiles, &set, rect, PixelFormat::RGBA8, None, &Interrupt::NONE).unwrap().to_interleaved(rect), [255; 16]);
    }

    /// Multiple tilesets share one decoded budget; reject aggregate overflow before decoding the next image.
    #[test]
    fn render_tilemaps_rejects_combined_tileset_overflow_before_inflating_the_second() {
        use photocraft_doc::sprite::SpriteFrame;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
        encoder.write_all(&[7; 16]).unwrap();
        let compressed = encoder.finish().unwrap();
        let run = |budget: usize, second: &[u8]| {
            let chunk = |bytes: Vec<u8>| SpriteChunk { kind: 0x2023, target: MetadataTarget::Sprite, bytes: Arc::new(bytes) };
            let empty = || SpriteFrame { duration_ms: 0, palette: Arc::new(Vec::new()), cels: Default::default(), metadata: Vec::new() };
            let mut frame = empty();
            let mut second = record(second);
            second[..4].copy_from_slice(&1u32.to_le_bytes());
            frame.metadata = vec![chunk(record(&compressed)), chunk(second)];
            let ctl = Interrupt::NONE;
            let mut decoder = Decoder {
                nodes: Vec::new(),
                parents: Vec::new(),
                layers: Vec::new(),
                frames: vec![Arc::new(frame)],
                frame: empty(),
                target: MetadataTarget::Sprite,
                fmt: PixelFormat::RGBA8,
                indexed: false,
                transparent: 0,
                flags: 0,
                budget,
                metadata_bytes: 0,
                chunks: 0,
                profile: None,
                profile_chunk: None,
                profile_seen: false,
                palette_seen: false,
                tags: Vec::new(),
                warnings: Default::default(),
                ctl: &ctl,
            };
            decoder.render_tilemaps().map(|()| decoder.budget)
        };
        let error = run(31, b"not zlib").unwrap_err().to_string();
        assert!(error.contains("remaining decoded memory budget"), "{error}");
        assert_eq!(run(32, &compressed).unwrap(), 32, "both temporary reservations are released after rendering");
        assert!(run(32, b"not zlib").unwrap_err().to_string().contains("invalid tile data"));
    }
}
