use std::{collections::BTreeMap, sync::Arc};

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    Document, Layer, LayerContent, Timeline,
    sprite::{self, MetadataTarget, PaletteEntry, SpriteCel, SpriteFrame, SpriteLayer, SpriteTag},
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::{Interrupt, Surface};

use super::{
    ImportResult, IoError, MAX_CHUNKS, MAX_FILE, MAX_LAYERS, MAX_PIXELS, Result, blend_mode, invalid,
    reader::{self, Reader},
};

pub(super) mod metadata;
pub(super) mod tilemap;

struct Node {
    layer: Layer,
    parent: Option<usize>,
}

struct Decoder<'a> {
    nodes: Vec<Node>,
    parents: Vec<usize>,
    layers: Vec<SpriteLayer>,
    frames: Vec<Arc<SpriteFrame>>,
    frame: SpriteFrame,
    target: MetadataTarget,
    fmt: PixelFormat,
    indexed: bool,
    transparent: u8,
    flags: u32,
    budget: usize,
    metadata_bytes: usize,
    chunks: usize,
    profile: Option<Arc<Vec<u8>>>,
    profile_chunk: Option<Arc<Vec<u8>>>,
    profile_seen: bool,
    palette_seen: bool,
    tags: Vec<SpriteTag>,
    warnings: crate::ora::Warnings,
    ctl: &'a Interrupt<'a>,
}

pub(super) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult> {
    if bytes.len() > MAX_FILE {
        return Err(invalid("file exceeds 512 MiB"));
    }
    let mut r = Reader::new(bytes);
    if r.u32()? as usize != bytes.len() || r.u16()? != 0xa5e0 {
        return Err(invalid("invalid file size or signature"));
    }
    let count = usize::from(r.u16()?);
    if count == 0 || count > sprite::MAX_FRAMES {
        return Err(invalid("invalid sprite frame count (maximum 10000)"));
    }
    let (w, h, depth) = (r.u16()?, r.u16()?, r.u16()?);
    if w == 0 || h == 0 {
        return Err(invalid("canvas dimensions must be positive"));
    }
    let mode = match depth {
        32 => ColorMode::Rgb,
        16 => ColorMode::Grayscale,
        8 => ColorMode::Indexed,
        _ => return Err(invalid("unsupported pixel depth")),
    };
    let flags = r.u32()?;
    if flags & !7 != 0 {
        return Err(invalid("unsupported header flags"));
    }
    r.take(10)?; // legacy speed/reserved
    let transparent = r.u8()?;
    r.take(3)?;
    let colors = usize::from(r.u16()?);
    let pixel_aspect = [r.u8()?, r.u8()?];
    let grid = [r.i16()?, r.i16()?, r.i16()?, r.i16()?];
    r.take(84)?;
    let indexed = mode == ColorMode::Indexed;
    let palette = Arc::new(if indexed { vec![PaletteEntry::default(); if colors == 0 { 256 } else { colors }] } else { Vec::new() });
    if palette.len() > 256 {
        return Err(invalid("indexed palette exceeds 256 entries"));
    }
    let empty_frame = |palette| SpriteFrame { duration_ms: 0, palette, cels: BTreeMap::new(), metadata: Vec::new() };
    let mut d = Decoder {
        nodes: Vec::new(),
        parents: Vec::new(),
        layers: Vec::new(),
        frames: Vec::new(),
        frame: empty_frame(palette),
        target: MetadataTarget::Sprite,
        fmt: PixelFormat::new(if indexed { ColorMode::Rgb } else { mode }, SampleType::U8, true),
        indexed,
        transparent,
        flags,
        budget: MAX_PIXELS,
        metadata_bytes: 0,
        chunks: 0,
        profile: None,
        profile_chunk: None,
        profile_seen: false,
        palette_seen: false,
        tags: Vec::new(),
        warnings: Default::default(),
        ctl,
    };
    for frame_index in 0..count {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let len = (r.u32()? as usize).checked_sub(16).ok_or_else(|| invalid("frame is shorter than its header"))?;
        if r.u16()? != 0xf1fa {
            return Err(invalid("invalid frame signature"));
        }
        let old_count = usize::from(r.u16()?);
        d.frame.duration_ms = r.u16()?;
        r.take(2)?;
        let new_count = r.u32()? as usize;
        let chunks = if new_count == 0 { old_count } else { new_count };
        let mut frame = Reader::new(r.take(len)?);
        if chunks > frame.remaining() / 6 {
            return Err(invalid("invalid frame chunk count"));
        }
        d.chunks = d.chunks.checked_add(chunks).ok_or_else(|| invalid("chunk count overflow"))?;
        if d.chunks > MAX_CHUNKS {
            return Err(invalid("sprite chunk count exceeds 65536"));
        }
        let mut records = Vec::with_capacity(chunks);
        for _ in 0..chunks {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            let size = (frame.u32()? as usize).checked_sub(6).ok_or_else(|| invalid("chunk is shorter than its header"))?;
            let kind = frame.u16()?;
            let data = frame.take(size)?;
            records.push((kind, data));
        }
        frame.end()?;
        let modern_palette = records.iter().any(|(kind, _)| *kind == 0x2019);
        for (kind, data) in records {
            if modern_palette && matches!(kind, 0x0004 | 0x0011) {
                continue;
            }
            d.chunk(kind, data, frame_index)?;
        }
        if d.indexed {
            for (id, cel) in &mut d.frame.cels {
                let cel = Arc::make_mut(cel);
                if let Some(indices) = &cel.indices {
                    let background = d.layers.iter().find(|l| l.id == *id).is_some_and(|l| l.flags & 8 != 0);
                    cel.pixels =
                        sprite::pixels::render_indices(indices, cel.rect, &d.frame.palette, (!background).then_some(d.transparent)).map_err(invalid)?;
                }
            }
        }
        let next = empty_frame(d.frame.palette.clone());
        d.frames.push(Arc::new(std::mem::replace(&mut d.frame, next)));
        d.target = MetadataTarget::Frame;
        ctl.progress((frame_index + 1) as f32 / count as f32 * 0.8);
    }
    r.end()?;
    if indexed && !d.palette_seen {
        return Err(invalid("indexed sprite is missing its palette"));
    }
    d.render_tilemaps()?;
    d.resolve_links()?;
    let sequence = sprite::Sprite {
        frames: d.frames,
        current: 0,
        layers: d.layers,
        indexed,
        transparent_index: transparent,
        pixel_aspect,
        grid,
        tags: d.tags,
        profile: d.profile_chunk,
    };
    sequence.validate().map_err(invalid)?;
    let mut doc = Document::new(name, Size::new(u32::from(w), u32::from(h)), mode, SampleType::U8);
    doc.icc_profile = d.profile;
    while let Some(mut node) = d.nodes.pop() {
        if let LayerContent::Group(g) = &mut node.layer.content {
            g.children.reverse()
        }
        match node.parent {
            Some(i) => d.nodes.get_mut(i).and_then(|p| p.layer.children_mut()).ok_or_else(|| invalid("invalid parent layer"))?.push(node.layer),
            None => doc.layers.push(node.layer),
        }
    }
    doc.layers.reverse();
    doc.timeline = Some(Timeline::new(count, 10.0));
    doc.sprite = Some(sequence);
    doc.restore_sprite_frame().map_err(invalid)?;
    Ok(ImportResult { document: doc, warnings: d.warnings.finish(), source_read_only: false, preview_only: false })
}

impl Decoder<'_> {
    fn chunk(&mut self, kind: u16, data: &[u8], frame: usize) -> Result<()> {
        let mut r = Reader::new(data);
        match kind {
            0x2004 => {
                if frame != 0 {
                    return Err(invalid("layer definitions must be in the first frame"));
                }
                self.layer(&mut r)?;
                if data.get(2..4) == Some(&[2, 0]) {
                    self.retain_tile_record(kind, data, self.target.clone())?;
                }
            }
            0x2005 => {
                self.cel(&mut r)?;
                if data.get(7..9) == Some(&[3, 0]) {
                    self.retain_tile_record(kind, data, self.target.clone())?;
                }
            }
            0x2006 => self.precise(&mut r)?,
            0x2007 => self.profile(data, &mut r)?,
            0x0004 | 0x0011 | 0x2019 => self.palette(kind, &mut r)?,
            0x2018 => self.tags(&mut r)?,
            0x2008 | 0x2016 | 0x2017 | 0x2020 | 0x2022 => {
                if kind == 0x2008 {
                    self.warnings.note("External Aseprite references were preserved; no external files were read")
                }
                self.retain(kind, data)?;
                r.take(r.remaining())?;
            }
            0x2023 => {
                if frame != 0 {
                    return Err(invalid("tileset definitions must be in the first frame"));
                }
                self.retain_tile_record(kind, data, MetadataTarget::Frame)?;
                r.take(r.remaining())?;
                self.target = MetadataTarget::Frame;
            }
            _ => return Err(invalid(format!("unsupported chunk type 0x{kind:04x}"))),
        }
        r.end()
    }

    fn charge(&mut self, rect: Rect, bpp: usize) -> Result<()> {
        let n = usize::try_from(rect.tile_count()).ok().and_then(|n| n.checked_mul(256 * 256 * bpp)).ok_or_else(|| invalid("cel tile size overflow"))?;
        self.budget = self.budget.checked_sub(n).ok_or_else(|| invalid("decoded sprite tiles exceed 256 MiB"))?;
        Ok(())
    }

    fn layer(&mut self, r: &mut Reader<'_>) -> Result<()> {
        if self.nodes.len() >= MAX_LAYERS {
            return Err(invalid("layer count exceeds 4096"));
        }
        let (flags, kind, level) = (r.u16()?, r.u16()?, usize::from(r.u16()?));
        if flags & !127 != 0 || kind > 2 {
            return Err(invalid("unsupported layer flags or tilemap layer"));
        }
        if kind == 2 && flags & 8 != 0 {
            return Err(invalid("a background flag on a tilemap cannot be represented safely"));
        }
        if level > photocraft_doc::MAX_GROUP_DEPTH || level > self.parents.len() {
            return Err(invalid("invalid or excessive group nesting"));
        }
        self.parents.truncate(level);
        let parent = self.parents.last().copied();
        r.take(4)?;
        let blend = r.u16()?;
        let opacity = r.u8()?;
        r.take(3)?;
        let name = r.string()?;
        let mut layer = if kind == 1 { Layer::group(name, Vec::new()) } else { Layer::raster(name, self.fmt) };
        layer.visible = flags & 1 != 0;
        layer.locks.all = flags & 2 == 0 || flags & 64 != 0 || kind == 2;
        layer.locks.position = flags & (4 | 8) != 0;
        layer.locks.transparency = flags & 8 != 0;
        if kind != 1 || self.flags & 2 != 0 {
            layer.blend = blend_mode(blend)?;
            if self.flags & 1 != 0 {
                layer.opacity = f32::from(opacity) / 255.0
            }
        }
        if let LayerContent::Group(g) = &mut layer.content {
            g.expanded = flags & 32 == 0;
            self.parents.push(self.nodes.len());
        }
        if kind == 2 {
            r.u32()?;
        }
        let uuid = if self.flags & 4 != 0 { Some(r.take(16)?.try_into().map_err(|_| invalid("invalid layer UUID"))?) } else { None };
        self.layers.push(SpriteLayer { id: layer.id, flags, opacity: (layer.opacity * 255.0).round() as u8, uuid });
        self.target = MetadataTarget::Layer(layer.id);
        self.nodes.push(Node { layer, parent });
        Ok(())
    }

    fn cel(&mut self, r: &mut Reader<'_>) -> Result<()> {
        let index = usize::from(r.u16()?);
        let (x, y, opacity, kind, z_index) = (i32::from(r.i16()?), i32::from(r.i16()?), r.u8()?, r.u16()?, r.i16()?);
        r.take(5)?;
        let node = self.nodes.get(index).ok_or_else(|| invalid("cel refers to a missing layer"))?;
        let id = node.layer.id;
        if node.layer.is_group() || self.frame.cels.contains_key(&id) {
            return Err(invalid("duplicate cel or cel on a group"));
        }
        let tile_layer = self
            .frame
            .metadata
            .iter()
            .chain(self.frames.first().into_iter().flat_map(|f| f.metadata.iter()))
            .any(|c| c.kind == 0x2004 && c.target == MetadataTarget::Layer(id));
        if (kind == 3 && !tile_layer) || (tile_layer && !matches!(kind, 1 | 3)) {
            return Err(invalid("cel type does not match the tilemap layer"));
        }
        let (rect, linked_frame, pixels, indices) = if kind == 1 {
            (Rect::from_xywh(x, y, 1, 1), Some(usize::from(r.u16()?)), Surface::new(self.fmt), None)
        } else if matches!(kind, 0 | 2) {
            let (w, h) = (u32::from(r.u16()?), u32::from(r.u16()?));
            let rect = Rect::from_xywh(x, y, w, h);
            sprite::pixels::checked_rect(rect, if self.indexed { 5 } else { self.fmt.bytes_per_pixel() }).map_err(invalid)?;
            self.charge(rect, if self.indexed { 5 } else { self.fmt.bytes_per_pixel() })?;
            let fmt = if self.indexed { PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false) } else { self.fmt };
            let mut pixels = Surface::new(fmt);
            reader::pixels(r, kind == 2, rect, &mut pixels, self.ctl)?;
            if self.indexed { (rect, None, Surface::new(self.fmt), Some(pixels)) } else { (rect, None, pixels, None) }
        } else if kind == 3 {
            r.take(r.remaining())?;
            (Rect::from_xywh(x, y, 1, 1), None, Surface::new(self.fmt), None)
        } else {
            return Err(invalid("unsupported tilemap cel"));
        };
        self.frame.cels.insert(id, Arc::new(SpriteCel { rect, pixels, indices, opacity, z_index, linked_frame, precise: None }));
        self.target = MetadataTarget::Cel(id);
        Ok(())
    }

    fn resolve_links(&mut self) -> Result<()> {
        let mut resolved = std::collections::BTreeSet::new();
        let keys: Vec<_> = self.frames.iter().enumerate().flat_map(|(f, frame)| frame.cels.keys().map(move |id| (f, *id))).collect();
        for (frame, id) in keys {
            self.ctl.check().map_err(|_| IoError::Cancelled)?;
            let mut path = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            let mut at = frame;
            while !resolved.contains(&(at, id)) {
                if !seen.insert(at) {
                    return Err(invalid("linked cels form a cycle"));
                }
                let cel = self.frames.get(at).and_then(|f| f.cels.get(&id)).ok_or_else(|| invalid("linked cel refers to a missing image"))?;
                match cel.linked_frame {
                    Some(next) => {
                        path.push(at);
                        at = next;
                    }
                    None => {
                        resolved.insert((at, id));
                        break;
                    }
                }
            }
            for dest in path.into_iter().rev() {
                let cel = self.frames.get(dest).and_then(|f| f.cels.get(&id)).ok_or_else(|| invalid("linked cel is missing"))?.clone();
                let source_frame = cel.linked_frame.ok_or_else(|| invalid("linked cel target is missing"))?;
                let source = self.frames.get(source_frame).and_then(|f| f.cels.get(&id)).ok_or_else(|| invalid("linked image is missing"))?.clone();
                let (dx, dy) = (cel.rect.x0 - source.rect.x0, cel.rect.y0 - source.rect.y0);
                let rect = Rect::from_xywh(cel.rect.x0, cel.rect.y0, source.rect.width(), source.rect.height());
                let same_palette = self.frames.get(dest).map(|f| &f.palette) == self.frames.get(source_frame).map(|f| &f.palette);
                if dx % 256 != 0 || dy % 256 != 0 || (self.indexed && !same_palette) {
                    self.charge(rect, if self.indexed { 5 } else { self.fmt.bytes_per_pixel() })?
                }
                let indices = source.indices.as_ref().map(|s| s.translated(dx, dy, source.rect));
                let frame = Arc::make_mut(self.frames.get_mut(dest).ok_or_else(|| invalid("linked frame is missing"))?);
                let background = self.layers.iter().find(|l| l.id == id).is_some_and(|l| l.flags & 8 != 0);
                let pixels = if self.indexed && !same_palette {
                    sprite::pixels::render_indices(
                        indices.as_ref().ok_or_else(|| invalid("linked indexed image has no indices"))?,
                        rect,
                        &frame.palette,
                        (!background).then_some(self.transparent),
                    )
                    .map_err(invalid)?
                } else {
                    source.pixels.translated(dx, dy, source.rect)
                };
                frame.cels.insert(id, Arc::new(SpriteCel { rect, pixels, indices, ..cel.as_ref().clone() }));
                resolved.insert((dest, id));
            }
        }
        Ok(())
    }
}
