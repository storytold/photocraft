use std::{collections::BTreeMap, io::Write, sync::Arc};

use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{
    Document, Layer, LayerContent, LayerId,
    sprite::{self, MetadataTarget, Sprite, SpriteCel, SpriteChunk, SpriteFrame, SpriteLayer},
};
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;

use super::{ExportResult, IoError, MAX_FILE, MAX_LAYERS, Result, blend_mode, invalid, reader::Reader};

pub(super) fn export(doc: &Document, ctl: &Interrupt<'_>) -> Result<ExportResult> {
    let losses = super::losses(doc);
    if !losses.is_empty() {
        return Err(invalid(losses.join("; ")));
    }
    if doc.depth != SampleType::U8 || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Indexed) {
        return Err(invalid("Aseprite stores 8-bit RGB, grayscale or indexed pixels; convert a copy before export"));
    }
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 65535 || doc.size.height > 65535 {
        return Err(invalid("Aseprite canvas dimensions must be 1..65535"));
    }
    let mut copy = doc.clone();
    copy.capture_sprite_frame().map_err(invalid)?;
    let synthesized;
    let sprite = match &copy.sprite {
        Some(s) => s,
        None => {
            synthesized = from_document(&copy)?;
            &synthesized
        }
    };
    sprite.validate().map_err(invalid)?;
    validate_tilemaps(sprite, copy.mode, ctl)?;
    let layers = layers(&copy, sprite)?;
    if layers.len() > MAX_LAYERS {
        return Err(invalid("layer count exceeds 4096"));
    }
    let pass = layers.iter().filter(|(l, _)| l.is_group() && l.blend == BlendMode::PassThrough).count();
    let groups = layers.iter().filter(|(l, _)| l.is_group()).count();
    if pass != 0 && pass != groups {
        return Err(invalid("Aseprite cannot encode mixed pass-through/isolated groups; use a converted copy"));
    }
    let uuids = sprite.layers.iter().any(|l| l.uuid.is_some());
    let flags = 1 | if pass == 0 { 2 } else { 0 } | if uuids { 4 } else { 0 };
    let mut bytes = vec![0; 128];
    word(&mut bytes, 4, 0xa5e0)?;
    word(&mut bytes, 6, u16::try_from(sprite.frames.len()).map_err(|_| invalid("too many frames"))?)?;
    word(&mut bytes, 8, copy.size.width as u16)?;
    word(&mut bytes, 10, copy.size.height as u16)?;
    word(
        &mut bytes,
        12,
        match copy.mode {
            ColorMode::Indexed => 8,
            ColorMode::Grayscale => 16,
            _ => 32,
        },
    )?;
    put(&mut bytes, 14, &u32::to_le_bytes(flags))?;
    word(&mut bytes, 18, sprite.frames.first().map_or(100, |f| f.duration_ms))?;
    put(&mut bytes, 28, &[sprite.transparent_index])?;
    put(&mut bytes, 34, &sprite.pixel_aspect)?;
    for (i, n) in sprite.grid.iter().enumerate() {
        word(&mut bytes, 36 + i * 2, *n as u16)?
    }
    let warnings = Vec::new();
    let mut chunk_count = 0usize;
    for (frame_index, frame) in sprite.frames.iter().enumerate() {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let mut chunks = Vec::new();
        if frame_index == 0
            && let Some(profile) = profile(&copy, sprite)?
        {
            chunks.push(chunk(0x2007, profile)?)
        }
        // Aseprite associates sprite user data only before any layer has become the
        // current object. Put palette/sprite data first, even for RGB documents.
        let palette_changed = frame_index == 0 || sprite.frames.get(frame_index - 1).is_none_or(|previous| previous.palette != frame.palette);
        if !frame.palette.is_empty() && (palette_changed || frame.metadata.iter().any(|m| m.target == MetadataTarget::Palette)) {
            chunks.push(palette(&frame.palette)?);
            append_metadata(&mut chunks, &frame.metadata, &MetadataTarget::Palette)?;
        }
        append_metadata(&mut chunks, &frame.metadata, &MetadataTarget::Sprite)?;
        append_metadata(&mut chunks, &frame.metadata, &MetadataTarget::Frame)?;
        if frame_index == 0 {
            for (layer, level) in &layers {
                chunks.push(layer_chunk(layer, *level, sprite, flags, copy.is_background(layer.id), ctl)?);
                append_metadata(&mut chunks, &frame.metadata, &MetadataTarget::Layer(layer.id))?;
            }
        }
        if frame_index == 0 && !sprite.tags.is_empty() {
            chunks.push(tags(sprite)?);
            for f in &sprite.frames {
                append_metadata(&mut chunks, &f.metadata, &MetadataTarget::Tags)?
            }
        }
        for (index, (layer, _)) in layers.iter().enumerate() {
            if layer.is_group() && frame.cels.contains_key(&layer.id) {
                return Err(invalid("a group layer cannot own a sprite cel"));
            }
            let Some(cel) = frame.cels.get(&layer.id) else { continue };
            let tilemap = frame.metadata.iter().find(|m| m.kind == 0x2005 && m.target == MetadataTarget::Cel(layer.id)).map(|m| m.bytes.as_slice());
            chunks.push(cel_chunk(index, cel, sprite.indexed, tilemap, ctl)?);
            if let Some(precise) = cel.precise {
                let mut data = 1u32.to_le_bytes().to_vec();
                for n in precise {
                    data.extend(n.to_le_bytes())
                }
                data.extend([0; 16]);
                chunks.push(chunk(0x2006, data)?);
            }
            append_metadata(&mut chunks, &frame.metadata, &MetadataTarget::Cel(layer.id))?;
        }
        chunk_count = chunk_count.checked_add(chunks.len()).ok_or_else(|| invalid("encoded chunk count overflow"))?;
        if chunk_count > super::MAX_CHUNKS {
            return Err(invalid("encoded sprite exceeds 65536 chunks"));
        }
        let length = chunks.iter().try_fold(16usize, |n, c| n.checked_add(c.len()).ok_or_else(|| invalid("frame size overflow")))?;
        if bytes.len().checked_add(length).is_none_or(|n| n > MAX_FILE) {
            return Err(invalid("encoded sprite exceeds 512 MiB"));
        }
        bytes.extend(u32::try_from(length).map_err(|_| invalid("frame is too large"))?.to_le_bytes());
        bytes.extend(0xf1fau16.to_le_bytes());
        bytes.extend(if chunks.len() > 65535 { 65535u16 } else { chunks.len() as u16 }.to_le_bytes());
        bytes.extend(frame.duration_ms.to_le_bytes());
        bytes.extend([0; 2]);
        bytes.extend(u32::try_from(chunks.len()).map_err(|_| invalid("too many chunks"))?.to_le_bytes());
        for c in chunks {
            bytes.extend(c)
        }
        ctl.progress((frame_index + 1) as f32 / sprite.frames.len() as f32);
    }
    let len = u32::try_from(bytes.len()).map_err(|_| invalid("sprite is too large"))?;
    put(&mut bytes, 0, &len.to_le_bytes())?;
    Ok(ExportResult { bytes, warnings })
}

fn from_document(doc: &Document) -> Result<Sprite> {
    let indexed = doc.mode == ColorMode::Indexed;
    let mut palette = Arc::new(if indexed {
        let table = doc.color_table.as_ref().ok_or_else(|| invalid("indexed document has no color table"))?;
        if table.colors.is_empty() || table.colors.len() > 256 {
            return Err(invalid("indexed color table must have 1..256 entries"));
        }
        table.colors.iter().map(|rgb| sprite::PaletteEntry { rgba: [rgb[0], rgb[1], rgb[2], 255], name: String::new() }).collect()
    } else {
        Vec::new()
    });
    let mut transparent = doc.color_table.as_ref().and_then(|t| t.transparent).unwrap_or(0);
    let mut cels = BTreeMap::new();
    let mut layers = Vec::new();
    for (_, _, l) in doc.walk() {
        let background = indexed && doc.color_table.as_ref().is_some_and(|t| t.transparent.is_none()) && doc.background().is_some_and(|bg| bg.id == l.id);
        layers.push(SpriteLayer { id: l.id, flags: if background { 11 } else { 3 }, opacity: (l.opacity.clamp(0.0, 1.0) * 255.0).round() as u8, uuid: None });
        match &l.content {
            LayerContent::Group(_) => {}
            LayerContent::Raster(pixels) => {
                let rect = pixels.content_bounds();
                if !rect.is_empty() {
                    let mut cel = SpriteCel {
                        rect,
                        pixels: if indexed { photocraft_raster::Surface::new(pixels.format()) } else { pixels.clone() },
                        indices: None,
                        opacity: 255,
                        z_index: 0,
                        linked_frame: None,
                        precise: None,
                    };
                    if indexed {
                        if doc.color_table.as_ref().is_some_and(|t| t.transparent.is_none()) {
                            require_opaque(pixels, rect)?;
                        }
                        sprite::pixels::capture_indices(&mut cel, pixels, &palette, doc.color_table.as_ref().and_then(|t| t.transparent)).map_err(invalid)?;
                    }
                    cels.insert(l.id, Arc::new(cel));
                }
            }
            _ => return Err(invalid("Aseprite stores raster layers and groups; rasterize text, vector and adjustment layers in a copy first")),
        }
    }
    if indexed && doc.color_table.as_ref().is_some_and(|t| t.transparent.is_none()) {
        transparent = reserve_transparent_index(Arc::make_mut(&mut palette), &layers, &cels)?;
    }
    Ok(Sprite {
        frames: vec![Arc::new(SpriteFrame { duration_ms: 100, palette, cels, metadata: Vec::new() })],
        current: 0,
        layers,
        indexed,
        transparent_index: transparent,
        pixel_aspect: [1, 1],
        grid: [0; 4],
        tags: Vec::new(),
        profile: None,
    })
}

/// Without a transparent entry, quantizing alpha would silently turn erased pixels opaque.
fn require_opaque(pixels: &photocraft_raster::Surface, rect: Rect) -> Result<()> {
    sprite::pixels::checked_rect(rect, 5).map_err(invalid)?;
    if pixels.format().alpha {
        for y in rect.y0..rect.y1 {
            let row = pixels.to_interleaved(Rect::from_xywh(rect.x0, y, rect.width(), 1));
            if row.chunks_exact(pixels.format().bytes_per_pixel()).any(|p| p.last().copied() != Some(255)) {
                return Err(invalid(
                    "indexed pixels have transparency but the color table has no transparent entry; add palette transparency before exporting",
                ));
            }
        }
    }
    Ok(())
}

// Background is a layer-wide flag. Check each frame's used pixels once per shared tile
// region, excluding transparent padding outside the cel rectangle.
fn require_opaque_background(sprite: &Sprite, id: LayerId, ctl: &Interrupt<'_>) -> Result<()> {
    let mut checked = std::collections::HashSet::new();
    for frame in &sprite.frames {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let Some(cel) = frame.cels.get(&id) else { continue };
        let pixels = &cel.pixels;
        let bpp = pixels.format().bytes_per_pixel();
        let mut occupied = 0u64;
        for (coord, tile) in pixels.tiles() {
            let tile_rect = coord.rect();
            let used = cel.rect.intersect(&tile_rect);
            if used.is_empty() {
                continue;
            }
            occupied += 1;
            let bounds = [used.x0 - tile_rect.x0, used.y0 - tile_rect.y0, used.x1 - tile_rect.x0, used.y1 - tile_rect.y0];
            if !checked.insert((Arc::as_ptr(tile) as usize, bpp, bounds)) {
                continue;
            }
            ctl.check().map_err(|_| IoError::Cancelled)?;
            for y in bounds[1]..bounds[3] {
                let start = (y as usize * 256 + bounds[0] as usize) * bpp;
                let end = (y as usize * 256 + bounds[2] as usize) * bpp;
                let row = tile.bytes().get(start..end).ok_or_else(|| invalid("invalid Background tile length"))?;
                if row.chunks_exact(bpp).any(|p| p.last().copied() != Some(255)) {
                    return Err(invalid(
                        "Aseprite Background layers must be opaque in every frame; convert the Background to an ordinary layer before exporting",
                    ));
                }
            }
        }
        if occupied < cel.rect.tile_count() && pixels.default_pixel().last().copied() != Some(1.0) {
            return Err(invalid("Aseprite Background layers must be opaque in every frame; convert the Background to an ordinary layer before exporting"));
        }
    }
    Ok(())
}

fn reserve_transparent_index(palette: &mut Vec<sprite::PaletteEntry>, layers: &[SpriteLayer], cels: &BTreeMap<LayerId, Arc<SpriteCel>>) -> Result<u8> {
    // Aseprite reserves the header index on every non-Background layer, even if the
    // source has no transparency. Reserve an unused entry so opaque index zero stays ink.
    let mut used = [false; 256];
    for layer in layers.iter().filter(|l| l.flags & 8 == 0) {
        if let Some(cel) = cels.get(&layer.id)
            && let Some(indices) = &cel.indices
        {
            for y in cel.rect.y0..cel.rect.y1 {
                for index in indices.to_interleaved(Rect::from_xywh(cel.rect.x0, y, cel.rect.width(), 1)) {
                    if let Some(value) = used.get_mut(usize::from(index)) {
                        *value = true;
                    }
                }
            }
        }
    }
    match used.iter().take(palette.len()).position(|v| !v) {
        Some(index) => Ok(index as u8),
        None if palette.len() < 256 => {
            let index = palette.len() as u8;
            palette.push(sprite::PaletteEntry { rgba: [0; 4], name: String::new() });
            Ok(index)
        }
        None => Err(invalid("Aseprite needs an unused transparent palette index on non-Background layers; reduce the palette or save a .pcraft copy")),
    }
}

fn layers<'a>(doc: &'a Document, sprite: &Sprite) -> Result<Vec<(&'a Layer, usize)>> {
    fn walk<'a>(layers: &'a [Layer], sprite: &Sprite, depth: usize, out: &mut Vec<(&'a Layer, usize)>) -> Result<()> {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(invalid("group depth limit exceeded"));
        }
        let mut ordered: Vec<_> = layers.iter().collect();
        ordered.sort_by_key(|l| sprite.layers.iter().position(|s| s.id == l.id).unwrap_or(sprite.layers.len()));
        for l in ordered {
            out.push((l, depth));
            if let Some(c) = l.children() {
                walk(c, sprite, depth + 1, out)?
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(&doc.layers, sprite, 0, &mut out)?;
    Ok(out)
}

fn layer_chunk(layer: &Layer, level: usize, sprite: &Sprite, flags: u32, background: bool, ctl: &Interrupt<'_>) -> Result<Vec<u8>> {
    let source = sprite.layers.iter().find(|l| l.id == layer.id);
    let retained = source.map_or(0, |s| s.flags) & (8 | 16 | 64);
    let mut bits = retained;
    if !sprite.indexed && sprite.tilemap_descriptor(layer.id).is_none() {
        // Layer From Background clears the lock; its retained file flag must not restore it.
        bits &= !8;
        if background || retained & 8 != 0 && layer.locks.transparency {
            bits |= 8;
        }
    }
    if bits & 8 != 0 {
        require_opaque_background(sprite, layer.id, ctl)?;
    }
    if layer.visible {
        bits |= 1
    }
    if !layer.locks.all || (bits & 64 != 0 || sprite.tilemap_descriptor(layer.id).is_some()) && source.is_some_and(|s| s.flags & 2 != 0) {
        bits |= 2
    }
    if layer.locks.position {
        bits |= 4
    }
    if layer.children().is_some_and(|_| matches!(&layer.content, LayerContent::Group(g) if !g.expanded)) {
        bits |= 32
    }
    let mut data = Vec::new();
    let blend = if layer.blend == BlendMode::PassThrough {
        0
    } else {
        (0u16..19).find(|i| blend_mode(*i).ok() == Some(layer.blend)).ok_or_else(|| invalid("layer blend mode cannot be represented in Aseprite"))?
    };
    let tilemap = sprite.tilemap_descriptor(layer.id).map(super::decode::tilemap::layer_tileset).transpose()?;
    for n in [bits, if tilemap.is_some() { 2 } else { u16::from(layer.is_group()) }, level as u16, 0, 0, blend] {
        data.extend(n.to_le_bytes())
    }
    let opacity = if layer.is_group() { (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8 } else { source.map_or(255, |s| s.opacity) };
    data.extend([opacity, 0, 0, 0]);
    string(&mut data, &layer.name)?;
    if let Some(index) = tilemap {
        data.extend(u32::try_from(index).map_err(|_| invalid("tileset index overflow"))?.to_le_bytes());
    }
    if flags & 4 != 0 {
        data.extend(source.and_then(|s| s.uuid).unwrap_or([0; 16]))
    }
    chunk(0x2004, data)
}

fn cel_chunk(index: usize, cel: &SpriteCel, indexed: bool, tilemap: Option<&[u8]>, ctl: &Interrupt<'_>) -> Result<Vec<u8>> {
    let mut data = u16::try_from(index).map_err(|_| invalid("layer index overflow"))?.to_le_bytes().to_vec();
    for n in [cel.rect.x0, cel.rect.y0] {
        data.extend(i16::try_from(n).map_err(|_| invalid("cel position exceeds Aseprite's signed 16-bit range"))?.to_le_bytes())
    }
    data.push(cel.opacity);
    data.extend(
        if cel.linked_frame.is_some() {
            1u16
        } else if tilemap.is_some() {
            3u16
        } else {
            2u16
        }
        .to_le_bytes(),
    );
    data.extend(cel.z_index.to_le_bytes());
    data.extend([0; 5]);
    if let Some(link) = cel.linked_frame {
        data.extend(u16::try_from(link).map_err(|_| invalid("linked frame index overflow"))?.to_le_bytes());
    } else if let Some(raw) = tilemap {
        // validate_tilemaps already decoded and checked this record before encoding.
        data.extend(raw.get(16..).ok_or_else(|| invalid("truncated tilemap record"))?);
    } else {
        data.extend(u16::try_from(cel.rect.width()).map_err(|_| invalid("cel width overflow"))?.to_le_bytes());
        data.extend(u16::try_from(cel.rect.height()).map_err(|_| invalid("cel height overflow"))?.to_le_bytes());
        let surface = if indexed { cel.indices.as_ref().ok_or_else(|| invalid("indexed cel has no indices"))? } else { &cel.pixels };
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        for y in cel.rect.y0..if cel.rect.is_empty() { cel.rect.y0 } else { cel.rect.y1 } {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            z.write_all(&surface.to_interleaved(Rect::from_xywh(cel.rect.x0, y, cel.rect.width(), 1))).map_err(|e| invalid(e.to_string()))?;
        }
        data.extend(z.finish().map_err(|e| invalid(e.to_string()))?);
    }
    chunk(0x2005, data)
}

fn palette(entries: &[sprite::PaletteEntry]) -> Result<Vec<u8>> {
    let mut data = (entries.len() as u32).to_le_bytes().to_vec();
    data.extend(0u32.to_le_bytes());
    data.extend((entries.len().saturating_sub(1) as u32).to_le_bytes());
    data.extend([0; 8]);
    for entry in entries {
        data.extend(u16::from(!entry.name.is_empty()).to_le_bytes());
        data.extend(entry.rgba);
        if !entry.name.is_empty() {
            string(&mut data, &entry.name)?
        }
    }
    chunk(0x2019, data)
}

fn tags(sprite: &Sprite) -> Result<Vec<u8>> {
    let mut data = u16::try_from(sprite.tags.len()).map_err(|_| invalid("too many animation tags"))?.to_le_bytes().to_vec();
    data.extend([0; 8]);
    for tag in &sprite.tags {
        data.extend((tag.from as u16).to_le_bytes());
        data.extend((tag.to as u16).to_le_bytes());
        data.push(tag.direction);
        data.extend(tag.repeat.to_le_bytes());
        data.extend([0; 6]);
        data.extend(tag.color);
        data.push(0);
        string(&mut data, &tag.name)?;
    }
    chunk(0x2018, data)
}

fn profile(doc: &Document, sprite: &Sprite) -> Result<Option<Vec<u8>>> {
    if let Some(raw) = &sprite.profile {
        let expected = super::decode::metadata::color_profile(&mut Reader::new(raw), doc.mode == ColorMode::Grayscale)?;
        if expected == doc.icc_profile {
            return Ok(Some(raw.as_ref().clone()));
        }
    }
    let Some(icc) = &doc.icc_profile else { return Ok(None) };
    if icc.len() > 16 << 20 {
        return Err(invalid("ICC profile exceeds 16 MiB"));
    }
    let mut data = 2u16.to_le_bytes().to_vec();
    data.extend([0; 14]);
    data.extend((icc.len() as u32).to_le_bytes());
    data.extend(icc.as_slice());
    super::decode::metadata::color_profile(&mut Reader::new(&data), doc.mode == ColorMode::Grayscale)?;
    Ok(Some(data))
}

fn append_metadata(chunks: &mut Vec<Vec<u8>>, metadata: &[SpriteChunk], target: &MetadataTarget) -> Result<()> {
    let mut records = metadata.iter().filter(|c| &c.target == target).peekable();
    while let Some(c) = records.next() {
        if matches!(c.kind, 0x2004 | 0x2005) {
            continue;
        }
        if c.kind != 0x2023 {
            super::decode::metadata::validate::validate(c.kind, &c.bytes)?;
        }
        chunks.push(chunk(c.kind, c.bytes.as_ref().clone())?);
        if c.kind == 0x2023 && records.peek().is_some_and(|m| m.kind == 0x2020) {
            // Once tileset user data is present the application also consumes one
            // user-data chunk per tile. Older independent writers omit empty entries.
            let mut r = Reader::new(&c.bytes);
            r.take(8)?;
            let count = r.u32()? as usize;
            if count >= super::MAX_CHUNKS {
                return Err(invalid("tileset user-data sequence exceeds the chunk limit"));
            }
            for _ in 0..=count {
                if records.peek().is_some_and(|m| m.kind == 0x2020) {
                    let data = records.next().ok_or_else(|| invalid("tileset user data is missing"))?;
                    super::decode::metadata::validate::validate(data.kind, &data.bytes)?;
                    chunks.push(chunk(data.kind, data.bytes.as_ref().clone())?);
                } else {
                    chunks.push(chunk(0x2020, vec![0; 4])?);
                }
            }
        }
    }
    Ok(())
}

fn validate_tilemaps(sprite: &Sprite, mode: ColorMode, ctl: &Interrupt<'_>) -> Result<()> {
    use super::decode::tilemap;
    let fmt = photocraft_color::PixelFormat::new(if sprite.indexed { ColorMode::Grayscale } else { mode }, SampleType::U8, !sprite.indexed);
    let mut sets = std::collections::BTreeMap::new();
    let mut bytes = 0usize;
    for (i, frame) in sprite.frames.iter().enumerate() {
        for c in &frame.metadata {
            if c.kind == 0x2023 {
                if i != 0 {
                    return Err(invalid("tilesets must be defined in the first frame"));
                }
                let set = tilemap::tileset(&c.bytes, fmt.bytes_per_pixel(), sprite::MAX_BYTES.saturating_sub(bytes), ctl)?;
                bytes = bytes.checked_add(set.pixels.as_ref().map_or(0, Vec::len)).ok_or_else(|| invalid("tileset size overflow"))?;
                if bytes > sprite::MAX_BYTES {
                    return Err(invalid("tilesets exceed 256 MiB"));
                }
                if sets.insert(set.id as usize, set).is_some() {
                    return Err(invalid("duplicate tileset index"));
                }
            }
        }
    }
    for frame in &sprite.frames {
        for c in &frame.metadata {
            if c.kind == 0x2005 {
                ctl.check().map_err(|_| IoError::Cancelled)?;
                let MetadataTarget::Cel(id) = c.target else { return Err(invalid("tilemap attachment is invalid")) };
                let descriptor = sprite.tilemap_descriptor(id).ok_or_else(|| invalid("tilemap cel has no layer descriptor"))?;
                let set = sets.get(&tilemap::layer_tileset(descriptor)?).ok_or_else(|| invalid("tilemap layer's tileset is missing"))?;
                let cel = frame.cels.get(&id).ok_or_else(|| invalid("tilemap cel is missing"))?;
                let (rect, pixels) =
                    tilemap::render(&tilemap::map(&c.bytes, ctl)?, set, (cel.rect.x0, cel.rect.y0), fmt, sprite.indexed.then_some(sprite.transparent_index))?;
                let stored = if sprite.indexed { cel.indices.as_ref().ok_or_else(|| invalid("tilemap index image is missing"))? } else { &cel.pixels };
                if rect != cel.rect || pixels != *stored {
                    return Err(invalid("tilemap display was edited without rasterization; original tile records were preserved"));
                }
            }
        }
    }
    Ok(())
}
fn string(out: &mut Vec<u8>, s: &str) -> Result<()> {
    out.extend(u16::try_from(s.len()).map_err(|_| invalid("string exceeds 65535 UTF-8 bytes"))?.to_le_bytes());
    out.extend(s.as_bytes());
    Ok(())
}
fn chunk(kind: u16, data: Vec<u8>) -> Result<Vec<u8>> {
    let n = data.len().checked_add(6).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| invalid("chunk size overflow"))?;
    let mut out = n.to_le_bytes().to_vec();
    out.extend(kind.to_le_bytes());
    out.extend(data);
    Ok(out)
}
fn word(out: &mut [u8], at: usize, n: u16) -> Result<()> {
    put(out, at, &n.to_le_bytes())
}
fn put(out: &mut [u8], at: usize, data: &[u8]) -> Result<()> {
    let end = at.checked_add(data.len()).ok_or_else(|| invalid("header offset overflow"))?;
    out.get_mut(at..end).ok_or_else(|| invalid("invalid header field"))?.copy_from_slice(data);
    Ok(())
}
