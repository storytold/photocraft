//! Editable cel animation. Pixel indices are retained separately from the RGB display cache;
//! selecting a frame never reconstructs indices from colors. This complements the ordinary
//! layer-visibility frame-animation model: a sprite owns different pixels in the same layer.

use std::{collections::BTreeMap, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::{LayerId, Rect, Surface};

mod edit;
pub mod pixels;
mod playback;
mod reference;
#[cfg(test)]
mod tests;

pub const MAX_FRAMES: usize = 10_000;
pub const MAX_CELS: usize = 65_536;
pub const MAX_BYTES: usize = 256 << 20;
pub const MAX_METADATA: usize = 32 << 20;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PaletteEntry {
    pub rgba: [u8; 4],
    pub name: String,
}

impl Default for PaletteEntry {
    fn default() -> Self {
        Self { rgba: [0, 0, 0, 255], name: String::new() }
    }
}

/// Metadata attaches to an object, rather than a transient chunk/layer array offset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetadataTarget {
    Sprite,
    Layer(LayerId),
    Cel(LayerId),
    Tags,
    Palette,
    Frame,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteChunk {
    pub kind: u16,
    pub target: MetadataTarget,
    pub bytes: Arc<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpriteLayer {
    pub id: LayerId,
    /// Original flags, including background/reference/prefer-linked properties.
    pub flags: u16,
    /// The layer opacity before the cel's opacity is applied to the display layer.
    pub opacity: u8,
    pub uuid: Option<[u8; 16]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteCel {
    /// The complete source rectangle, including transparent borders and off-canvas samples.
    pub rect: Rect,
    /// Straight-alpha samples in document coordinates, shared by linked cels until edited.
    pub pixels: Surface,
    /// Grayscale/U8/no-alpha index samples, also in document coordinates. Authoritative in
    /// indexed sprites; `pixels` is its display cache, never the source of unchanged indices.
    pub indices: Option<Surface>,
    pub opacity: u8,
    pub z_index: i16,
    pub linked_frame: Option<usize>,
    /// Original precise reference bounds (16.16 fixed point), when supplied by the format.
    pub precise: Option<[i32; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpriteFrame {
    pub duration_ms: u16,
    pub palette: Arc<Vec<PaletteEntry>>,
    /// Missing entry means an empty cel, distinct from an image of transparent samples.
    pub cels: BTreeMap<LayerId, Arc<SpriteCel>>,
    pub metadata: Vec<SpriteChunk>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpriteTag {
    pub name: String,
    pub from: usize,
    pub to: usize,
    /// 0 forward, 1 reverse, 2 ping-pong, 3 reverse ping-pong.
    pub direction: u8,
    /// Zero repeats indefinitely.
    pub repeat: u16,
    pub color: [u8; 3],
}

#[derive(Clone, Debug)]
pub struct Sprite {
    pub frames: Vec<Arc<SpriteFrame>>,
    pub current: usize,
    /// Original bottom-to-top, depth-first order, independent of the selected cel z-order.
    pub layers: Vec<SpriteLayer>,
    pub indexed: bool,
    pub transparent_index: u8,
    pub pixel_aspect: [u8; 2],
    pub grid: [i16; 4],
    pub tags: Vec<SpriteTag>,
    /// Color profile payload, retained for fixed-gamma/ICC round trips.
    pub profile: Option<Arc<Vec<u8>>>,
}

impl PartialEq for Sprite {
    fn eq(&self, other: &Self) -> bool {
        let Self { frames, current, layers, indexed, transparent_index, pixel_aspect, grid, tags, profile } = self;
        frames.len() == other.frames.len()
            && frames.iter().zip(&other.frames).all(|(a, b)| Arc::ptr_eq(a, b) || a == b)
            && *current == other.current
            && *layers == other.layers
            && *indexed == other.indexed
            && *transparent_index == other.transparent_index
            && *pixel_aspect == other.pixel_aspect
            && *grid == other.grid
            && *tags == other.tags
            && *profile == other.profile
    }
}

impl Sprite {
    /// Validate native/command-created state before any projection, editing or export.
    /// Counts distinct allocated tiles, including boundary tiles and linked image storage.
    pub fn validate(&self) -> Result<(), String> {
        if self.frames.is_empty() || self.frames.len() > MAX_FRAMES || self.current >= self.frames.len() || self.layers.len() > 4096 {
            return Err("invalid sprite frame/layer count or selected frame".into());
        }
        let ids: std::collections::BTreeSet<_> = self.layers.iter().map(|l| l.id).collect();
        if ids.len() != self.layers.len() {
            return Err("duplicate sprite layer id".into());
        }
        let mut bytes = 0usize;
        let mut metadata = 0usize;
        let mut count = 0usize;
        let mut chunk_count = 0usize;
        let mut tiles = std::collections::HashSet::new();
        let mut palettes = std::collections::HashSet::new();
        let mut index_tiles = std::collections::HashSet::new();
        for frame in &self.frames {
            let mut projected = 0usize;
            count = count.checked_add(frame.cels.len()).ok_or("sprite cel count overflow")?;
            chunk_count = chunk_count.checked_add(frame.metadata.len()).ok_or("sprite chunk count overflow")?;
            if count > MAX_CELS || chunk_count > MAX_CELS || frame.palette.len() > 256 || (self.indexed && frame.palette.is_empty()) {
                return Err("sprite cel/palette limit exceeded".into());
            }
            if palettes.insert(Arc::as_ptr(&frame.palette)) {
                for entry in frame.palette.iter() {
                    if entry.name.len() > 65_535 {
                        return Err("sprite palette name is too long".into());
                    }
                    metadata = metadata.checked_add(entry.name.len()).ok_or("sprite metadata size overflow")?;
                }
            }
            for chunk in &frame.metadata {
                if match chunk.target {
                    MetadataTarget::Layer(id) => !ids.contains(&id),
                    MetadataTarget::Cel(id) => !frame.cels.contains_key(&id),
                    _ => false,
                } {
                    return Err("sprite metadata refers to a missing object".into());
                }
                metadata = metadata.checked_add(chunk.bytes.len()).ok_or("sprite metadata size overflow")?;
            }
            if metadata > MAX_METADATA {
                return Err("sprite metadata exceeds 32 MiB".into());
            }
            for (id, cel) in &frame.cels {
                if !ids.contains(id) || cel.rect.x1 < cel.rect.x0 || cel.rect.y1 < cel.rect.y0 || cel.rect.width() > 65_535 || cel.rect.height() > 65_535 {
                    return Err("invalid sprite cel layer or rectangle".into());
                }
                pixels::checked_rect(cel.rect, if self.indexed { 5 } else { cel.pixels.format().bytes_per_pixel() })?;
                let fmt = cel.pixels.format();
                if !matches!(fmt.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
                    || fmt.sample != photocraft_color::SampleType::U8
                    || !fmt.alpha
                {
                    return Err("invalid sprite pixel surface format".into());
                }
                if cel.precise.is_some_and(|b| b[2] <= 0 || b[3] <= 0) {
                    return Err("invalid precise cel dimensions".into());
                }
                if let Some(bounds) = cel.precise {
                    let rect = reference::bounds(bounds);
                    pixels::checked_rect(rect, fmt.bytes_per_pixel())?;
                    let cost = usize::try_from(rect.tile_count())
                        .ok()
                        .and_then(|n| n.checked_mul(256 * 256 * fmt.bytes_per_pixel()))
                        .ok_or("reference projection size overflow")?;
                    projected = projected.checked_add(cost).ok_or("reference projection size overflow")?;
                    if projected > MAX_BYTES {
                        return Err("reference display tiles exceed 256 MiB".into());
                    }
                }
                if cel.linked_frame.is_some_and(|f| !self.frames.get(f).is_some_and(|f| f.cels.contains_key(id))) {
                    return Err("sprite linked cel refers to a missing cel in the same layer".into());
                }
                if cel.indices.is_some() != self.indexed {
                    return Err("sprite index plane does not match its color mode".into());
                }
                for surface in std::iter::once(&cel.pixels).chain(cel.indices.iter()) {
                    for (_, tile) in surface.tiles() {
                        if tiles.insert(Arc::as_ptr(tile)) {
                            let cost = 256 * 256 * surface.format().bytes_per_pixel();
                            bytes = bytes.checked_add(cost).ok_or("sprite tile budget overflow")?;
                        }
                    }
                    if bytes > MAX_BYTES {
                        return Err("sprite decoded tiles exceed 256 MiB".into());
                    }
                }
                if let Some(indices) = &cel.indices {
                    let f = indices.format();
                    if f != photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U8, false) {
                        return Err("invalid sprite index-plane format".into());
                    }
                    validate_indices(indices, cel.rect, frame.palette.len(), &mut index_tiles)?;
                }
            }
        }
        if self.tags.len() > 65535 {
            return Err("sprite tag count exceeds 65535".into());
        }
        for tag in &self.tags {
            if tag.from > tag.to || tag.to >= self.frames.len() || tag.direction > 3 || tag.name.len() > 65_535 {
                return Err("invalid sprite animation tag".into());
            }
            metadata = metadata.checked_add(tag.name.len() + std::mem::size_of::<SpriteTag>()).ok_or("sprite tag size overflow")?;
        }
        if metadata > MAX_METADATA {
            return Err("sprite metadata exceeds 32 MiB".into());
        }
        if self.profile.as_ref().is_some_and(|p| p.len() > (16 << 20) + 20) {
            return Err("sprite profile exceeds 16 MiB".into());
        }
        // Resolve each link chain once, including forward links. A cycle never reaches an
        // image. The memo bounds total traversal by the cel count, without recursion.
        let mut resolved = std::collections::BTreeSet::new();
        for (frame_index, frame) in self.frames.iter().enumerate() {
            for id in frame.cels.keys() {
                let mut path = std::collections::BTreeSet::new();
                let mut at = frame_index;
                while !resolved.contains(&(at, *id)) {
                    if !path.insert((at, *id)) {
                        return Err("sprite linked cels form a cycle".into());
                    }
                    match self.frames.get(at).and_then(|f| f.cels.get(id)).and_then(|c| c.linked_frame) {
                        Some(next) => at = next,
                        None => break,
                    }
                }
                resolved.extend(path);
            }
        }
        Ok(())
    }

    pub fn frame_duration_ms(&self) -> u16 {
        self.frames.get(self.current).map_or(0, |f| f.duration_ms)
    }

    /// A retained tilemap descriptor. The raster surface is its display cache, not a
    /// replacement for tile ids, masks or the tileset reference stored in this record.
    pub fn tilemap_descriptor(&self, id: LayerId) -> Option<&[u8]> {
        self.frames.first().and_then(|f| f.metadata.iter().find(|c| c.kind == 0x2004 && c.target == MetadataTarget::Layer(id))).map(|c| c.bytes.as_slice())
    }

    pub fn duration_ms(&self) -> u64 {
        self.frames.iter().map(|f| u64::from(f.duration_ms)).sum()
    }

    /// Remap every stable layer reference when a native document is opened or duplicated.
    /// A missing target is an error: silently reattaching metadata would corrupt the sprite.
    pub fn remap_layers(&mut self, map: impl Fn(LayerId) -> Option<LayerId>) -> Result<(), String> {
        for layer in &mut self.layers {
            layer.id = map(layer.id).ok_or("sprite refers to a missing layer")?;
        }
        for frame in &mut self.frames {
            let frame = Arc::make_mut(frame);
            frame.cels =
                frame.cels.iter().map(|(id, cel)| Ok((map(*id).ok_or("sprite cel refers to a missing layer")?, cel.clone()))).collect::<Result<_, String>>()?;
            for chunk in &mut frame.metadata {
                match &mut chunk.target {
                    MetadataTarget::Layer(id) | MetadataTarget::Cel(id) => *id = map(*id).ok_or("sprite metadata refers to a missing layer")?,
                    _ => {}
                }
            }
        }
        self.validate()
    }
}

/// Check samples actually used by a cel, including sparse default pixels. Shared tiles and
/// full 256-entry palettes need no repeated scan while validating linked animation frames.
fn validate_indices(
    indices: &Surface,
    rect: Rect,
    palette_len: usize,
    checked: &mut std::collections::HashSet<(usize, usize, [i32; 4])>,
) -> Result<(), String> {
    if palette_len == 256 || rect.is_empty() {
        return Ok(());
    }
    let mut occupied = 0u64;
    for (tc, tile) in indices.tiles() {
        let tile_rect = tc.rect();
        let used = rect.intersect(&tile_rect);
        if used.is_empty() {
            continue;
        }
        occupied += 1;
        let bounds = [used.x0 - tile_rect.x0, used.y0 - tile_rect.y0, used.x1 - tile_rect.x0, used.y1 - tile_rect.y0];
        if !checked.insert((Arc::as_ptr(tile) as usize, palette_len, bounds)) {
            continue;
        }
        for y in bounds[1]..bounds[3] {
            let start = y as usize * 256 + bounds[0] as usize;
            let end = y as usize * 256 + bounds[2] as usize;
            let row = tile.bytes().get(start..end).ok_or("invalid sprite index tile length")?;
            if row.iter().any(|v| usize::from(*v) >= palette_len) {
                return Err("sprite pixel refers to a missing palette entry".into());
            }
        }
    }
    if occupied < rect.tile_count() && indices.default_pixel().first().is_some_and(|v| (*v * 255.0).round() as usize >= palette_len) {
        return Err("sprite default pixel refers to a missing palette entry".into());
    }
    Ok(())
}
