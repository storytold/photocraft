//! Native cel animation persistence. Pixels/index planes use the ordinary content-addressed
//! tile store; opaque metadata/profile payloads use bounded binary blobs, never JSON arrays.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use photocraft_doc::{
    LayerId,
    sprite::{MAX_BYTES, MAX_CELS, MAX_FRAMES, MAX_METADATA, Sprite, SpriteCel, SpriteChunk, SpriteFrame},
};

use super::{FormatError, Loader, Result, Sink, surface_m};
use crate::manifest::{SpriteCelM, SpriteChunkM, SpriteFrameM, SpriteM};

pub(super) fn encode(sprite: &Sprite, sink: &mut dyn Sink) -> SpriteM {
    let mut palettes = Vec::new();
    let mut palette_ids = HashMap::new();
    let frames = sprite
        .frames
        .iter()
        .map(|frame| {
            let palette = *palette_ids.entry(frame.palette.clone()).or_insert_with(|| {
                let id = palettes.len();
                palettes.push(frame.palette.as_ref().clone());
                id
            });
            SpriteFrameM {
                duration_ms: frame.duration_ms,
                palette,
                cels: frame
                    .cels
                    .iter()
                    .map(|(layer, cel)| SpriteCelM {
                        layer: layer.0,
                        rect: cel.rect,
                        pixels: surface_m(&cel.pixels, sink),
                        indices: cel.indices.as_ref().map(|s| surface_m(s, sink)),
                        opacity: cel.opacity,
                        z_index: cel.z_index,
                        linked_frame: cel.linked_frame,
                        precise: cel.precise,
                    })
                    .collect(),
                metadata: frame.metadata.iter().map(|c| SpriteChunkM { kind: c.kind, target: c.target.clone(), blob: sink.blob(&c.bytes) }).collect(),
            }
        })
        .collect();
    SpriteM {
        current: sprite.current,
        layers: sprite.layers.clone(),
        indexed: sprite.indexed,
        transparent_index: sprite.transparent_index,
        pixel_aspect: sprite.pixel_aspect,
        grid: sprite.grid,
        tags: sprite.tags.clone(),
        profile: sprite.profile.as_ref().map(|p| sink.blob(p)),
        palettes,
        frames,
    }
}

impl Loader<'_> {
    pub(super) fn sprite(&mut self, m: &SpriteM) -> Result<Sprite> {
        let bad = |message: &str| FormatError::corrupt(message);
        if m.frames.is_empty()
            || m.frames.len() > MAX_FRAMES
            || m.current >= m.frames.len()
            || m.layers.len() > 4096
            || m.palettes.len() > MAX_FRAMES
            || m.palettes.iter().any(|p| p.len() > 256)
        {
            return Err(bad("invalid sprite frame/palette/layer count"));
        }
        check_tile_budget(m)?;
        let palettes: Vec<_> = m.palettes.iter().map(|p| Arc::new(p.clone())).collect();
        let mut count = 0usize;
        let mut metadata_bytes = 0usize;
        let mut frames = Vec::with_capacity(m.frames.len());
        for frame in &m.frames {
            count = count.checked_add(frame.cels.len()).ok_or_else(|| bad("sprite cel count overflow"))?;
            if count > MAX_CELS || frame.metadata.len() > MAX_CELS {
                return Err(bad("sprite cel/metadata count limit exceeded"));
            }
            let palette = palettes.get(frame.palette).ok_or_else(|| bad("sprite palette reference is missing"))?.clone();
            let mut cels = BTreeMap::new();
            for cel in &frame.cels {
                let id = LayerId(cel.layer);
                if cels.contains_key(&id) {
                    return Err(bad("duplicate sprite cel layer"));
                }
                cels.insert(
                    id,
                    Arc::new(SpriteCel {
                        rect: cel.rect,
                        pixels: self.surface(&cel.pixels)?,
                        indices: cel.indices.as_ref().map(|s| self.surface(s)).transpose()?,
                        opacity: cel.opacity,
                        z_index: cel.z_index,
                        linked_frame: cel.linked_frame,
                        precise: cel.precise,
                    }),
                );
            }
            let mut metadata = Vec::with_capacity(frame.metadata.len());
            for c in &frame.metadata {
                let remaining = MAX_METADATA.checked_sub(metadata_bytes).ok_or_else(|| bad("sprite metadata exceeds 32 MiB"))?;
                let bytes = self.fetch.blob_bounded(&c.blob, remaining)?;
                metadata_bytes = metadata_bytes.checked_add(bytes.len()).ok_or_else(|| bad("sprite metadata size overflow"))?;
                metadata.push(SpriteChunk { kind: c.kind, target: c.target.clone(), bytes });
            }
            frames.push(Arc::new(SpriteFrame { duration_ms: frame.duration_ms, palette, cels, metadata }));
        }
        let mut sprite = Sprite {
            frames,
            current: m.current,
            layers: m.layers.clone(),
            indexed: m.indexed,
            transparent_index: m.transparent_index,
            pixel_aspect: m.pixel_aspect,
            grid: m.grid,
            tags: m.tags.clone(),
            profile: m.profile.as_ref().map(|p| self.fetch.blob_bounded(p, (16 << 20) + 20)).transpose()?,
        };
        sprite.remap_layers(|id| self.id_map.get(&id.0).copied()).map_err(FormatError::corrupt)?;
        Ok(sprite)
    }
}

// Hash/sample pairs match Loader::surface's allocation cache.
fn check_tile_budget(m: &SpriteM) -> Result<()> {
    let bad = |message: &str| FormatError::corrupt(message);
    // Preflight distinct tile allocations before fetching any cel surface. The bundle's
    // overall budget is much larger than a sprite's, and validation after decode is late.
    let mut tiles = std::collections::HashSet::new();
    let mut tile_bytes = 0usize;
    for surface in m.frames.iter().flat_map(|f| &f.cels).flat_map(|c| std::iter::once(&c.pixels).chain(c.indices.iter())) {
        for tile in &surface.tiles {
            if tiles.insert((surface.format.sample, &tile.hash)) {
                tile_bytes = tile_bytes.checked_add(super::tile_len(&surface.format)).ok_or_else(|| bad("sprite tile budget overflow"))?;
                if tile_bytes > MAX_BYTES {
                    return Err(bad("sprite decoded tiles exceed 256 MiB"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        convert::Fetch,
        manifest::{SpriteCelM, SurfaceM, TileRef},
    };
    use photocraft_color::PixelFormat;

    fn manifest(unique: bool) -> SpriteM {
        let cel = SpriteCelM {
            layer: 1,
            rect: photocraft_geom::Rect::from_xywh(0, 0, 8448, 8192),
            pixels: SurfaceM {
                format: PixelFormat::RGBA8,
                default: "00000000".into(),
                tiles: (0..1025).map(|i| TileRef { tx: i % 33, ty: i / 33, hash: format!("{:064x}", if unique { i } else { 0 }) }).collect(),
            },
            indices: None,
            opacity: 255,
            z_index: 0,
            linked_frame: None,
            precise: None,
        };
        SpriteM {
            current: 0,
            layers: vec![],
            indexed: false,
            transparent_index: 0,
            pixel_aspect: [1, 1],
            grid: [0; 4],
            tags: vec![],
            profile: None,
            palettes: vec![vec![]],
            frames: vec![SpriteFrameM { duration_ms: 100, palette: 0, cels: vec![cel], metadata: vec![] }],
        }
    }

    #[test]
    fn sprite_budget_is_checked_before_fetching_tiles_and_shared_hashes_count_once() {
        struct FetchSpy(usize);
        impl Fetch for FetchSpy {
            fn tile(&mut self, _: &str, _: usize) -> Result<Vec<u8>> {
                self.0 += 1;
                Err(FormatError::corrupt("unexpected tile fetch"))
            }
            fn blob(&mut self, _: &str) -> Result<Arc<Vec<u8>>> {
                Err(FormatError::corrupt("unexpected blob fetch"))
            }
            fn blob_bounded(&mut self, _: &str, _: usize) -> Result<Arc<Vec<u8>>> {
                Err(FormatError::corrupt("unexpected blob fetch"))
            }
        }
        let mut fetch = FetchSpy(0);
        let mut loader = Loader {
            fetch: &mut fetch,
            preserve_ids: true,
            id_map: Default::default(),
            contents_id_map: Default::default(),
            legacy_linked_contents: Default::default(),
            tiles: Default::default(),
        };
        assert!(loader.sprite(&manifest(true)).unwrap_err().to_string().contains("256 MiB"));
        assert_eq!(fetch.0, 0, "refuse the manifest before any tile is decompressed");
        let shared = manifest(false);
        check_tile_budget(&shared).unwrap();
        let mut exact = manifest(true);
        exact.frames[0].cels[0].pixels.tiles.pop();
        check_tile_budget(&exact).unwrap();
        let mut combined = exact.clone();
        combined.frames[0].cels[0].indices = Some(SurfaceM {
            format: PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U8, false),
            default: "00".into(),
            tiles: vec![TileRef { tx: 0, ty: 0, hash: format!("{:064x}", 1025) }],
        });
        assert!(check_tile_budget(&combined).unwrap_err().to_string().contains("256 MiB"), "index planes share the same budget");
        let mut repeated = exact.clone();
        repeated.frames.push(exact.frames[0].clone());
        check_tile_budget(&repeated).unwrap();
    }
}
