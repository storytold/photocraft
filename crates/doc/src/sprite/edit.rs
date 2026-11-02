//! Projection into ordinary editable layers, and capture back into the selected cel frame.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use photocraft_color::{ColorMode, PixelFormat, SampleType};

use super::{MetadataTarget, Sprite, SpriteCel, SpriteLayer, pixels};
use crate::{ColorTable, Document, Layer, LayerContent, LayerId, Surface};

fn order(layer: &Layer, sprite: &Sprite, frame: usize) -> (i64, i16) {
    let rank = sprite.layers.iter().position(|l| l.id == layer.id).unwrap_or(sprite.layers.len());
    let z = sprite.frames.get(frame).and_then(|f| f.cels.get(&layer.id)).map_or(0, |c| c.z_index);
    (rank as i64 + i64::from(z), z)
}

fn display_order(layers: &[Layer], sprite: &Sprite, frame: usize, depth: usize, out: &mut Vec<LayerId>) -> Result<(), String> {
    if depth > crate::MAX_GROUP_DEPTH {
        return Err("sprite group nesting limit exceeded".into());
    }
    let mut ordered: Vec<_> = layers.iter().collect();
    ordered.sort_by_key(|l| order(l, sprite, frame));
    for l in ordered {
        out.push(l.id);
        if let Some(children) = l.children() {
            display_order(children, sprite, frame, depth + 1, out)?
        }
    }
    Ok(())
}

fn project(layers: &mut [Layer], sprite: &Sprite, frame: usize, fmt: PixelFormat, depth: usize) -> Result<(), String> {
    if depth > crate::MAX_GROUP_DEPTH {
        return Err("sprite layer groups exceed the nesting limit".into());
    }
    let frame_data = sprite.frames.get(frame).ok_or("sprite frame is missing")?;
    layers.sort_by_key(|l| order(l, sprite, frame));
    for layer in layers {
        if let Some(children) = layer.children_mut() {
            project(children, sprite, frame, fmt, depth + 1)?;
        } else if matches!(layer.content, LayerContent::Raster(_)) {
            let base = sprite.layers.iter().find(|s| s.id == layer.id).map_or(1.0, |s| f32::from(s.opacity) / 255.0);
            match frame_data.cels.get(&layer.id) {
                Some(cel) => {
                    let reference = sprite.layers.iter().find(|l| l.id == layer.id).is_some_and(|l| l.flags & 64 != 0);
                    layer.content = LayerContent::Raster(if reference { super::reference::projection(cel)? } else { cel.pixels.clone() });
                    layer.opacity = base * f32::from(cel.opacity) / 255.0;
                }
                None => {
                    layer.content = LayerContent::Raster(Surface::new(fmt));
                    layer.opacity = base;
                }
            }
        }
    }
    Ok(())
}

impl Document {
    /// Commit the displayed raster edits before navigating/saving. The session calls this
    /// inside the transaction so a failed bounds/palette check cannot leave a partial edit.
    pub fn capture_sprite_frame(&mut self) -> Result<(), String> {
        let Some(mut sprite) = self.sprite.take() else { return Ok(()) };
        let result = self.capture_sprite(&mut sprite);
        self.sprite = Some(sprite);
        result
    }

    fn capture_sprite(&mut self, sprite: &mut Sprite) -> Result<(), String> {
        sprite.validate()?;
        let expected_mode = if self.mode == ColorMode::Grayscale { ColorMode::Grayscale } else { ColorMode::Rgb };
        if self.depth != SampleType::U8
            || sprite.indexed != (self.mode == ColorMode::Indexed)
            || !matches!(self.mode, ColorMode::Indexed | ColorMode::Rgb | ColorMode::Grayscale)
            || sprite.frames.iter().any(|f| f.cels.values().any(|c| c.pixels.format().mode != expected_mode))
        {
            return Err("convert a separate copy before changing a sprite's bit depth or indexed color mode".into());
        }
        let current = sprite.current;
        let mut expected_order = Vec::new();
        display_order(&self.layers, sprite, current, 0, &mut expected_order)?;
        let actual_order: Vec<_> = self.walk().iter().map(|(_, _, l)| l.id).collect();
        let reordered = actual_order != expected_order;
        let frame = sprite.frames.get(current).ok_or("sprite frame is missing")?.clone();
        let existing: BTreeSet<_> = self.walk().iter().map(|(_, _, l)| l.id).collect();
        sprite.layers.retain(|l| existing.contains(&l.id));
        // Explicit layer deletion removes that object's cels and attached metadata everywhere.
        for f in &mut sprite.frames {
            if f.cels.keys().any(|id| !existing.contains(id))
                || f.metadata.iter().any(|c| match c.target {
                    MetadataTarget::Layer(id) | MetadataTarget::Cel(id) => !existing.contains(&id),
                    _ => false,
                })
            {
                let f = Arc::make_mut(f);
                f.cels.retain(|id, _| existing.contains(id));
                f.metadata.retain(|c| match c.target {
                    MetadataTarget::Layer(id) | MetadataTarget::Cel(id) => existing.contains(&id),
                    _ => true,
                });
            }
        }
        let mut palette = frame.palette.clone();
        let old_transparent = sprite.transparent_index;
        if sprite.indexed
            && let Some(table) = &self.color_table
        {
            if table.transparent.is_none() {
                return Err("Aseprite retains a transparent palette index; keep it or detach a copy before removing palette transparency".into());
            }
            if table.colors.len() != palette.len() || table.colors.iter().zip(palette.iter()).any(|(a, b)| *a != [b.rgba[0], b.rgba[1], b.rgba[2]]) {
                let entries = Arc::make_mut(&mut palette);
                entries.resize_with(table.colors.len(), Default::default);
                for (entry, color) in entries.iter_mut().zip(&table.colors) {
                    entry.rgba[..3].copy_from_slice(color);
                }
            }
            if let Some(index) = table.transparent {
                if index != sprite.transparent_index && sprite.layers.iter().any(|l| sprite.tilemap_descriptor(l.id).is_some()) {
                    return Err("rasterize a copy of the tilemaps before changing the sprite's transparent palette index".into());
                }
                sprite.transparent_index = index;
            }
        }
        let mut changes = Vec::new();
        for (_, _, layer) in self.walk() {
            let tilemap = sprite.tilemap_descriptor(layer.id).is_some();
            let info = match sprite.layers.iter().position(|l| l.id == layer.id) {
                Some(i) => i,
                None => {
                    sprite.layers.push(SpriteLayer { id: layer.id, flags: 3, opacity: (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8, uuid: None });
                    sprite.layers.len() - 1
                }
            };
            let base = sprite.layers.get_mut(info).ok_or("sprite layer is missing")?;
            base.flags = (base.flags & !1) | u16::from(layer.visible);
            if layer.is_group() {
                base.opacity = (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                continue;
            }
            // A reference layer's surface is a scaled display cache, not its source image.
            if base.flags & 64 != 0 {
                if !layer.locks.all {
                    return Err("reference layers retain their original image and precise bounds; detach a copy before unlocking or painting them".into());
                }
                continue;
            }
            let LayerContent::Raster(surface) = &layer.content else { continue };
            let source = frame.cels.get(&layer.id);
            if tilemap && (source.is_some_and(|c| c.pixels != *surface) || source.is_none() && surface.tile_count() != 0) {
                return Err("rasterize a copy of this tilemap before editing its pixels; tile ids and tilesets were left unchanged".into());
            }
            let mut cel = source.map(|c| c.as_ref().clone()).unwrap_or_else(|| SpriteCel {
                rect: surface.content_bounds(),
                pixels: Surface::new(surface.format()),
                indices: None,
                opacity: 255,
                z_index: 0,
                linked_frame: None,
                precise: None,
            });
            if source.is_none() && surface.tile_count() == 0 {
                continue;
            }
            let expected = f32::from(base.opacity) / 255.0 * f32::from(cel.opacity) / 255.0;
            if (layer.opacity - expected).abs() > 1e-6 {
                // A layer Opacity edit changes the layer globally. Values outside the cel's
                // current range can be represented by opening that cel to full opacity.
                if cel.opacity == 0 || layer.opacity > f32::from(cel.opacity) / 255.0 {
                    cel.opacity = 255;
                }
                base.opacity = (layer.opacity.clamp(0.0, 1.0) * 255.0 * 255.0 / f32::from(cel.opacity.max(1))).round().clamp(0.0, 255.0) as u8;
            }
            let changed = source.is_none() || cel.pixels != *surface;
            if changed {
                if sprite.indexed {
                    let transparent = (base.flags & 8 == 0).then_some(sprite.transparent_index);
                    pixels::capture_indices(&mut cel, surface, &palette, transparent)?;
                } else {
                    cel.rect = cel.rect.union(&surface.content_bounds());
                    pixels::checked_rect(cel.rect, surface.format().bytes_per_pixel())?;
                    cel.pixels = surface.clone();
                }
                changes.push((layer.id, cel.clone()));
            } else if sprite.indexed
                && palette != frame.palette
                && let Some(indices) = &cel.indices
            {
                cel.pixels = pixels::render_indices(indices, cel.rect, &palette, (base.flags & 8 == 0).then_some(sprite.transparent_index))?;
            }
            if source.is_none() || source.is_some_and(|old| **old != cel) {
                Arc::make_mut(sprite.frames.get_mut(current).ok_or("sprite frame is missing")?).cels.insert(layer.id, Arc::new(cel));
            }
        }
        if palette != frame.palette {
            Arc::make_mut(sprite.frames.get_mut(current).ok_or("sprite frame is missing")?).palette = palette;
        }
        if reordered {
            let ranks: BTreeMap<_, _> = actual_order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
            sprite.layers.sort_by_key(|l| ranks.get(&l.id).copied().unwrap_or(usize::MAX));
            // A deliberate layer reorder becomes the selected frame's order; an old cel
            // z override must not immediately undo the user's drag.
            let frame = Arc::make_mut(sprite.frames.get_mut(current).ok_or("sprite frame is missing")?);
            for cel in frame.cels.values_mut() {
                if cel.z_index != 0 {
                    Arc::make_mut(cel).z_index = 0;
                }
            }
        }
        for (id, cel) in changes {
            sprite.update_linked_images(current, id, &frame.cels, &cel)?;
        }
        if sprite.indexed && old_transparent != sprite.transparent_index {
            for frame in &mut sprite.frames {
                let frame = Arc::make_mut(frame);
                for (id, cel) in &mut frame.cels {
                    let background = sprite.layers.iter().find(|l| l.id == *id).is_some_and(|l| l.flags & 8 != 0);
                    let cel = Arc::make_mut(cel);
                    if let Some(indices) = &cel.indices {
                        cel.pixels = pixels::render_indices(indices, cel.rect, &frame.palette, (!background).then_some(sprite.transparent_index))?;
                    }
                }
            }
        }
        sprite.validate()?;
        self.apply_sprite(sprite, current)
    }

    fn apply_sprite(&mut self, sprite: &Sprite, index: usize) -> Result<(), String> {
        let frame = sprite.frames.get(index).ok_or("sprite frame is missing")?;
        let fmt = PixelFormat::new(if self.mode == ColorMode::Grayscale { ColorMode::Grayscale } else { ColorMode::Rgb }, SampleType::U8, true);
        project(&mut self.layers, sprite, index, fmt, 0)?;
        if sprite.indexed {
            self.color_table = Some(ColorTable {
                colors: frame.palette.iter().map(|e| [e.rgba[0], e.rgba[1], e.rgba[2]]).collect(),
                transparent: Some(sprite.transparent_index),
            });
        }
        Ok(())
    }

    /// Navigation changes only the projection, not history or the underlying cel samples.
    pub fn select_sprite_frame(&mut self, index: usize) -> Result<(), String> {
        if !self.sprite.as_ref().is_some_and(|s| index < s.frames.len()) {
            return Err("sprite frame is out of range".into());
        }
        self.capture_sprite_frame()?;
        let Some(mut sprite) = self.sprite.take() else { return Err("document has no cel animation".into()) };
        let result = if index >= sprite.frames.len() {
            Err("sprite frame is out of range".into())
        } else {
            self.apply_sprite(&sprite, index).map(|()| {
                sprite.current = index;
                if let Some(t) = &mut self.timeline {
                    t.current = index;
                    t.duration = sprite.frames.len();
                    t.work_end = t.work_end.min(t.duration);
                }
            })
        };
        self.sprite = Some(sprite);
        result
    }

    /// Restore the selected projection after import or native loading, without capturing a
    /// stale display cache over authoritative cel data.
    pub fn restore_sprite_frame(&mut self) -> Result<(), String> {
        let Some(sprite) = self.sprite.take() else { return Ok(()) };
        let result = sprite.validate().and_then(|()| self.apply_sprite(&sprite, sprite.current));
        self.sprite = Some(sprite);
        result
    }
}

impl Sprite {
    fn root_frame(&self, mut frame: usize, id: LayerId) -> usize {
        for _ in 0..self.frames.len() {
            match self.frames.get(frame).and_then(|f| f.cels.get(&id)).and_then(|c| c.linked_frame) {
                Some(link) => frame = link,
                None => break,
            }
        }
        frame
    }

    fn update_linked_images(&mut self, current: usize, id: LayerId, old: &BTreeMap<LayerId, Arc<SpriteCel>>, edited: &SpriteCel) -> Result<(), String> {
        let root = self.root_frame(current, id);
        let old_rect = old.get(&id).map_or(edited.rect, |c| c.rect);
        let targets: Vec<_> = (0..self.frames.len()).filter(|f| *f != current && self.root_frame(*f, id) == root).collect();
        for f in targets {
            let frame = Arc::make_mut(self.frames.get_mut(f).ok_or("sprite linked frame is missing")?);
            let Some(cel) = frame.cels.get_mut(&id) else { continue };
            let cel = Arc::make_mut(cel);
            let (dx, dy) = (
                cel.rect.x0.checked_sub(old_rect.x0).ok_or("sprite cel offset overflow")?,
                cel.rect.y0.checked_sub(old_rect.y0).ok_or("sprite cel offset overflow")?,
            );
            cel.rect = edited.rect.translate(dx, dy);
            cel.indices = edited.indices.as_ref().map(|s| s.translated(dx, dy, edited.rect));
            cel.pixels = if let Some(indices) = &cel.indices {
                let background = self.layers.iter().find(|l| l.id == id).is_some_and(|l| l.flags & 8 != 0);
                pixels::render_indices(indices, cel.rect, &frame.palette, (!background).then_some(self.transparent_index))?
            } else {
                edited.pixels.translated(dx, dy, edited.rect)
            };
        }
        Ok(())
    }
}
