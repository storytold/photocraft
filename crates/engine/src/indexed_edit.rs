use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use photocraft_color::{ColorMode, PixelFormat, read_sample, write_sample};
use photocraft_doc::{ColorTable, Document, LayerContent};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, Tile};

const MAX_PIXEL_BYTES: usize = 16;
const CACHE_SIZE: usize = 16;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PixelBytes {
    len: u8,
    bytes: [u8; MAX_PIXEL_BYTES],
}

impl PixelBytes {
    fn from_slice(bytes: &[u8]) -> Self {
        let mut out = [0; MAX_PIXEL_BYTES];
        let len = bytes.len().min(MAX_PIXEL_BYTES);
        out[..len].copy_from_slice(&bytes[..len]);
        Self { len: len as u8, bytes: out }
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy)]
struct CacheEntry {
    key: PixelBytes,
    index: usize,
}

struct NearestCache {
    entries: [Option<CacheEntry>; CACHE_SIZE],
    next: usize,
}

impl NearestCache {
    fn new() -> Self {
        Self { entries: [None; CACHE_SIZE], next: 0 }
    }

    fn get(&self, key: PixelBytes) -> Option<usize> {
        self.entries.iter().flatten().find(|entry| entry.key == key).map(|entry| entry.index)
    }

    fn insert(&mut self, key: PixelBytes, index: usize) {
        self.entries[self.next] = Some(CacheEntry { key, index });
        self.next = (self.next + 1) % CACHE_SIZE;
    }
}

#[derive(Clone, Copy)]
struct PixelTarget {
    index: usize,
    alpha: f32,
}

fn indexed_error(message: impl Into<String>) -> crate::EngineError {
    crate::EngineError::Other(format!("indexed color constraint: {}", message.into()))
}

fn validate_table(table: &ColorTable) -> crate::Result<()> {
    if table.colors.is_empty() || table.colors.len() > 256 {
        return Err(indexed_error("color table must contain 1 to 256 entries"));
    }
    if let Some(index) = table.transparent
        && usize::from(index) >= table.colors.len()
    {
        return Err(indexed_error("transparent color-table index is outside the table"));
    }
    Ok(())
}

fn validate_surface(surface: &Surface, original: Option<&Surface>) -> crate::Result<PixelFormat> {
    let format = surface.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Indexed) || format.channels() > 4 || format.bytes_per_pixel() > MAX_PIXEL_BYTES {
        return Err(indexed_error("indexed raster surfaces must use RGB or RGBA storage"));
    }
    if let Some(original) = original
        && original.format() != format
    {
        return Err(indexed_error("indexed raster surface format changed during an edit"));
    }
    Ok(format)
}

fn nearest_opaque(table: &ColorTable, c: [f32; 3]) -> crate::Result<usize> {
    let mut best_distance = f32::MAX;
    let mut best_index = None;
    let mut first_opaque = None;
    for (index, entry) in table.colors.iter().enumerate() {
        if table.transparent == Some(index as u8) {
            continue;
        }
        first_opaque.get_or_insert(index);
        let distance = (0..3).map(|channel| (c[channel] * 255.0 - f32::from(entry[channel])).powi(2)).sum::<f32>();
        if distance < best_distance {
            best_distance = distance;
            best_index = Some(index);
        }
    }
    best_index.or(first_opaque).ok_or_else(|| indexed_error("opaque pixels cannot be represented because the color table has no opaque entry"))
}

fn source_rgb(bytes: &[u8], format: PixelFormat, base: usize) -> [f32; 3] {
    let sample_base = base / format.sample.bytes();
    [read_sample(bytes, format.sample, sample_base), read_sample(bytes, format.sample, sample_base + 1), read_sample(bytes, format.sample, sample_base + 2)]
}

fn source_alpha(bytes: &[u8], format: PixelFormat, base: usize) -> f32 {
    if format.alpha { read_sample(bytes, format.sample, base / format.sample.bytes() + 3) } else { 1.0 }
}

fn changed_target(bytes: &[u8], format: PixelFormat, table: &ColorTable, cache: &mut NearestCache, base: usize) -> crate::Result<PixelTarget> {
    let alpha = source_alpha(bytes, format, base);
    if table.transparent.is_some() && alpha < 0.5 {
        Ok(PixelTarget { index: usize::from(table.transparent.unwrap_or(0)), alpha: 0.0 })
    } else {
        let key = PixelBytes::from_slice(&bytes[base..base + format.bytes_per_pixel()]);
        let index = if let Some(index) = cache.get(key) {
            index
        } else {
            let index = nearest_opaque(table, source_rgb(bytes, format, base))?;
            cache.insert(key, index);
            index
        };
        Ok(PixelTarget { index, alpha: 1.0 })
    }
}

fn default_target(bytes: &[u8], format: PixelFormat, table: &ColorTable) -> crate::Result<PixelTarget> {
    let alpha = source_alpha(bytes, format, 0);
    if table.transparent.is_some() && alpha < 0.5 {
        Ok(PixelTarget { index: usize::from(table.transparent.unwrap_or(0)), alpha: 0.0 })
    } else {
        Ok(PixelTarget { index: nearest_opaque(table, source_rgb(bytes, format, 0))?, alpha: 1.0 })
    }
}

fn target_bytes(target: PixelTarget, format: PixelFormat, table: &ColorTable) -> PixelBytes {
    let mut out = [0; MAX_PIXEL_BYTES];
    let entry = table.colors.get(target.index).copied().unwrap_or([0; 3]);
    for (channel, value) in entry.into_iter().enumerate() {
        write_sample(&mut out, format.sample, channel, f32::from(value) / 255.0);
    }
    if format.alpha {
        write_sample(&mut out, format.sample, 3, target.alpha);
    }
    PixelBytes { len: format.bytes_per_pixel() as u8, bytes: out }
}

fn target_samples(target: PixelTarget, table: &ColorTable) -> [f32; 4] {
    let entry = table.colors.get(target.index).copied().unwrap_or([0; 3]);
    [f32::from(entry[0]) / 255.0, f32::from(entry[1]) / 255.0, f32::from(entry[2]) / 255.0, target.alpha]
}

fn uniform_pixel(tile: &Arc<Tile>, bpp: usize, cache: &mut HashMap<*const Tile, Option<PixelBytes>>) -> Option<PixelBytes> {
    let key = Arc::as_ptr(tile);
    if let Some(value) = cache.get(&key) {
        return *value;
    }
    let bytes = tile.bytes();
    let first = bytes.get(..bpp).map(PixelBytes::from_slice);
    let uniform = first.filter(|first| bytes.chunks_exact(bpp).all(|pixel| pixel == first.as_slice()));
    cache.insert(key, uniform);
    uniform
}

fn replace_default(surface: &mut Surface, target: PixelTarget, format: PixelFormat, table: &ColorTable) {
    let replacement = Surface::with_default(format, &target_samples(target, table)[..format.channels()]);
    let old = std::mem::replace(surface, replacement);
    for (coordinate, tile) in old.tiles() {
        surface.put_tiles(std::iter::once((*coordinate, tile.clone())));
    }
}
fn same_surface_storage(a: &Surface, b: &Surface) -> bool {
    a.format() == b.format()
        && a.default_bytes() == b.default_bytes()
        && a.tile_count() == b.tile_count()
        && a.tiles().zip(b.tiles()).all(|((ac, at), (bc, bt))| ac == bc && Arc::ptr_eq(at, bt))
}

fn old_pixel(tile: Option<&Arc<Tile>>, default: PixelBytes, base: usize, bpp: usize) -> PixelBytes {
    tile.and_then(|tile| tile.bytes().get(base..base + bpp).map(PixelBytes::from_slice)).unwrap_or(default)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChangeClass {
    None,
    All,
    Mixed,
}

fn change_class(
    after: PixelBytes,
    original: Option<&Arc<Tile>>,
    original_default: PixelBytes,
    bpp: usize,
    uniform: &mut HashMap<*const Tile, Option<PixelBytes>>,
) -> ChangeClass {
    let Some(tile) = original else {
        return if after == original_default { ChangeClass::None } else { ChangeClass::All };
    };
    if let Some(pixel) = uniform_pixel(tile, bpp, uniform) {
        return if pixel == after { ChangeClass::None } else { ChangeClass::All };
    }
    let mut equal = false;
    let mut different = false;
    for pixel in tile.bytes().chunks_exact(bpp) {
        if pixel == after.as_slice() {
            equal = true;
        } else {
            different = true;
        }
        if equal && different {
            return ChangeClass::Mixed;
        }
    }
    if different { ChangeClass::All } else { ChangeClass::None }
}

struct ConstraintContext<'a> {
    format: PixelFormat,
    table: &'a ColorTable,
    damage: Rect,
    uniform: HashMap<*const Tile, Option<PixelBytes>>,
    solids: HashMap<PixelBytes, Arc<Tile>>,
}

impl<'a> ConstraintContext<'a> {
    fn new(format: PixelFormat, table: &'a ColorTable, damage: Rect) -> Self {
        Self { format, table, damage, uniform: HashMap::new(), solids: HashMap::new() }
    }
}

fn process_removed_tile(
    surface: &mut Surface,
    coordinate: TileCoord,
    original: Arc<Tile>,
    after_default: PixelBytes,
    context: &mut ConstraintContext<'_>,
) -> crate::Result<()> {
    let tile_rect = coordinate.rect();
    let region = tile_rect.intersect(&context.damage);
    if region.is_empty() {
        return Ok(());
    }
    let bpp = context.format.bytes_per_pixel();
    let full_tile = region == tile_rect && tile_rect.width() == TILE_SIZE as u32 && tile_rect.height() == TILE_SIZE as u32;
    let class = if full_tile {
        let class = change_class(after_default, Some(&original), after_default, bpp, &mut context.uniform);
        if class == ChangeClass::None {
            return Ok(());
        }
        Some(class)
    } else {
        let original_bytes = original.bytes();
        let mut changed = false;
        for y in region.y0..region.y1 {
            let ly = y.saturating_sub(tile_rect.y0) as usize;
            for x in region.x0..region.x1 {
                let lx = x.saturating_sub(tile_rect.x0) as usize;
                let base = (ly * TILE_SIZE as usize + lx) * bpp;
                if PixelBytes::from_slice(&original_bytes[base..base + bpp]) != after_default {
                    changed = true;
                    break;
                }
            }
            if changed {
                break;
            }
        }
        if !changed {
            return Ok(());
        }
        None
    };
    let mut nearest = NearestCache::new();
    let target = changed_target(after_default.as_slice(), context.format, context.table, &mut nearest, 0)?;
    let encoded = target_bytes(target, context.format, context.table);
    if encoded == after_default {
        return Ok(());
    }
    if class == Some(ChangeClass::All) {
        let tile = if let Some(tile) = context.solids.get(&encoded) {
            tile.clone()
        } else {
            let tile = surface.solid_tile(&target_samples(target, context.table)[..context.format.channels()]);
            context.solids.insert(encoded, tile.clone());
            tile
        };
        surface.put_tiles(std::iter::once((coordinate, tile)));
        return Ok(());
    }
    let tile = surface.try_tile_mut(coordinate)?;
    let original_bytes = original.bytes();
    for y in region.y0..region.y1 {
        let ly = y.saturating_sub(tile_rect.y0) as usize;
        for x in region.x0..region.x1 {
            let lx = x.saturating_sub(tile_rect.x0) as usize;
            let base = (ly * TILE_SIZE as usize + lx) * bpp;
            let before = PixelBytes::from_slice(&original_bytes[base..base + bpp]);
            if before != after_default {
                tile.bytes_mut()[base..base + bpp].copy_from_slice(encoded.as_slice());
            }
        }
    }
    Ok(())
}
fn process_tile(
    surface: &mut Surface,
    coordinate: TileCoord,
    after: Arc<Tile>,
    original: Option<&Arc<Tile>>,
    original_default: PixelBytes,
    context: &mut ConstraintContext<'_>,
) -> crate::Result<()> {
    let tile_rect = coordinate.rect();
    let region = tile_rect.intersect(&context.damage);
    if region.is_empty() {
        return Ok(());
    }
    let bpp = context.format.bytes_per_pixel();
    let full_tile = region == tile_rect && tile_rect.width() == TILE_SIZE as u32 && tile_rect.height() == TILE_SIZE as u32;

    if full_tile && let Some(after_pixel) = uniform_pixel(&after, bpp, &mut context.uniform) {
        match change_class(after_pixel, original, original_default, bpp, &mut context.uniform) {
            ChangeClass::None => return Ok(()),
            ChangeClass::All => {
                let mut nearest = NearestCache::new();
                let target = changed_target(after.bytes(), context.format, context.table, &mut nearest, 0)?;
                let encoded = target_bytes(target, context.format, context.table);
                if encoded == after_pixel {
                    return Ok(());
                }
                let tile = if let Some(tile) = context.solids.get(&encoded) {
                    tile.clone()
                } else {
                    let tile = surface.solid_tile(&target_samples(target, context.table)[..context.format.channels()]);
                    context.solids.insert(encoded, tile.clone());
                    tile
                };
                surface.put_tiles(std::iter::once((coordinate, tile)));
                return Ok(());
            }
            ChangeClass::Mixed => {}
        }
    }

    let mut nearest = NearestCache::new();
    let mut changed = false;
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let after_bytes = after.bytes();
    for y in region.y0..region.y1 {
        let ly = y.saturating_sub(tile_rect.y0) as usize;
        for x in region.x0..region.x1 {
            let lx = x.saturating_sub(tile_rect.x0) as usize;
            let base = (ly * TILE_SIZE as usize + lx) * bpp;
            let pixel = PixelBytes::from_slice(&after_bytes[base..base + bpp]);
            if pixel == old_pixel(original, original_default, base, bpp) {
                continue;
            }
            let target = changed_target(after_bytes, context.format, context.table, &mut nearest, base)?;
            if target_bytes(target, context.format, context.table) != pixel {
                changed = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x.saturating_add(1));
                max_y = max_y.max(y.saturating_add(1));
            }
        }
    }
    if !changed {
        return Ok(());
    }

    let tile = surface.try_tile_mut(coordinate)?;
    let after_bytes = after.bytes();
    let mut nearest = NearestCache::new();
    for y in min_y..max_y {
        let ly = y.saturating_sub(tile_rect.y0) as usize;
        for x in min_x..max_x {
            let lx = x.saturating_sub(tile_rect.x0) as usize;
            let base = (ly * TILE_SIZE as usize + lx) * bpp;
            let pixel = PixelBytes::from_slice(&after_bytes[base..base + bpp]);
            if pixel == old_pixel(original, original_default, base, bpp) {
                continue;
            }
            let target = changed_target(after_bytes, context.format, context.table, &mut nearest, base)?;
            let encoded = target_bytes(target, context.format, context.table);
            if encoded != pixel {
                tile.bytes_mut()[base..base + bpp].copy_from_slice(encoded.as_slice());
            }
        }
    }
    Ok(())
}

fn layer_paths(doc: &Document) -> crate::Result<Vec<(Vec<usize>, usize)>> {
    let mut paths = Vec::new();
    let mut pending = vec![(Vec::new(), &doc.layers[..], 0usize)];
    while let Some((prefix, layers, depth)) = pending.pop() {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(indexed_error("layer nesting exceeds the supported depth"));
        }
        for (index, layer) in layers.iter().enumerate() {
            let mut path = prefix.clone();
            path.push(index);
            paths.push((path.clone(), depth));
            if let Some(children) = layer.children() {
                pending.push((path, children, depth + 1));
            }
        }
    }
    Ok(paths)
}

/// Constrain indexed pixel changes made by a staged document edit to its existing color table.
pub(crate) fn constrain_document(before: &Document, after: &mut Document) -> crate::Result<()> {
    if after.mode != ColorMode::Indexed {
        return Ok(());
    }
    // Entering Indexed Color, changing the Color Table, and changing depth already perform their
    // own explicit flattening/remapping. Re-constraining those transitions would apply the
    // mapping twice. A malformed or missing table is likewise tolerated until a changed raster
    // surface actually needs constraining.
    if before.mode != ColorMode::Indexed || before.depth != after.depth || before.color_table != after.color_table {
        return Ok(());
    }
    let table = after.color_table.clone();

    let mut originals = HashMap::new();
    let mut pending = vec![(&before.layers[..], 0usize)];
    while let Some((layers, depth)) = pending.pop() {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(indexed_error("layer nesting exceeds the supported depth"));
        }
        for layer in layers {
            if let LayerContent::Raster(surface) = &layer.content {
                originals.insert(layer.id, surface);
            }
            if let Some(children) = layer.children() {
                pending.push((children, depth + 1));
            }
        }
    }
    let damage = after.bounds();
    let paths = layer_paths(after)?;
    for (path, _) in paths {
        let (id, needs_work) = {
            let layer = after.layer_at(&path).ok_or_else(|| indexed_error("layer tree changed during indexed edit"))?;
            let LayerContent::Raster(surface) = &layer.content else { continue };
            let original = originals.get(&layer.id).copied();
            validate_surface(surface, original)?;
            let changed = !original.is_some_and(|original| same_surface_storage(surface, original));
            (layer.id, changed)
        };
        if !needs_work {
            continue;
        }
        let layer = after.layer_at_mut(&path).ok_or_else(|| indexed_error("layer tree changed during indexed edit"))?;
        if let LayerContent::Raster(surface) = &mut layer.content {
            let original = originals.get(&id).copied();
            let table = table.as_ref().ok_or_else(|| indexed_error("indexed document has no color table"))?;
            constrain_surface(surface, original, table, damage)?;
        }
    }
    Ok(())
}

/// Constrain changed pixels in one expanded indexed surface, preserving sparse tiles and padding.
pub(crate) fn constrain_surface(surface: &mut Surface, original: Option<&Surface>, table: &ColorTable, damage: Rect) -> crate::Result<()> {
    validate_table(table)?;
    let format = validate_surface(surface, original)?;
    if damage.is_empty() {
        return Ok(());
    }
    let after_default = PixelBytes::from_slice(surface.default_bytes());
    let original_default = original.map(|surface| PixelBytes::from_slice(surface.default_bytes())).unwrap_or(after_default);
    let default_changed = original.is_some() && original_default != after_default;
    let new_opaque_default = original.is_none() && (!format.alpha || source_alpha(after_default.as_slice(), format, 0) >= 0.5);
    if default_changed || new_opaque_default {
        let target = default_target(after_default.as_slice(), format, table)?;
        let encoded = target_bytes(target, format, table);
        if encoded != after_default {
            replace_default(surface, target, format, table);
        }
    }
    let original_tiles: Option<HashSet<*const Tile>> = original.map(|surface| surface.tiles().map(|(_, tile)| Arc::as_ptr(tile)).collect());
    let removed_coordinates: Vec<TileCoord> =
        if default_changed { Vec::new() } else { original.map(|surface| surface.tiles().map(|(coordinate, _)| *coordinate).collect()).unwrap_or_default() };

    let coordinates: Vec<TileCoord> = surface.tiles().map(|(coordinate, _)| *coordinate).collect();
    let mut context = ConstraintContext::new(format, table, damage);
    for coordinate in coordinates {
        let Some(after) = surface.tile(coordinate).cloned() else { continue };
        let original_tile = original.and_then(|surface| surface.tile(coordinate));
        if original_tiles.as_ref().is_some_and(|tiles| tiles.contains(&Arc::as_ptr(&after))) {
            continue;
        }
        process_tile(surface, coordinate, after, original_tile, original_default, &mut context)?;
    }
    if !default_changed && let Some(original) = original {
        for coordinate in removed_coordinates {
            if surface.tile(coordinate).is_none()
                && let Some(original_tile) = original.tile(coordinate).cloned()
            {
                process_removed_tile(surface, coordinate, original_tile, after_default, &mut context)?;
            }
        }
    }
    Ok(())
}
