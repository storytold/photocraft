//! Cel editing complements `timeline.*` navigation, without duplicating the independent
//! Photoshop layer-visibility `animation.*` command family under review in #6142.

use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    LayerId,
    sprite::{PaletteEntry, Sprite, SpriteTag},
};
use serde_json::{Value, json};
use std::sync::Arc;

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let doc = &s.active().ok_or("no document open")?.doc;
    if doc.sprite.is_none() {
        return Err("no cel animation".into());
    }
    if doc.depth != photocraft_color::SampleType::U8
        || !matches!(doc.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale | photocraft_color::ColorMode::Indexed)
    {
        return Err("Aseprite cel animation requires 8-bit RGB, grayscale or indexed samples".into());
    }
    Ok(())
}

fn cel_enabled(s: &Session) -> std::result::Result<(), String> {
    enabled(s)?;
    let state = s.active().ok_or("no document open")?;
    let id = state.active_layer.ok_or("no active layer")?;
    crate::commands::check_all_unlocked(&state.doc, id).map_err(|e| e.to_string())
}

fn check_tilemap_parent(doc: &photocraft_doc::Document, id: LayerId) -> Result<()> {
    let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
    let parents = path.get(..path.len().saturating_sub(1)).ok_or_else(|| bad("layer path is missing"))?;
    if doc.locks_at(parents).all {
        return Err(bad("unlock the parent group before rasterizing its tilemap"));
    }
    Ok(())
}

fn tilemap_enabled(s: &Session) -> std::result::Result<(), String> {
    enabled(s)?;
    let state = s.active().ok_or("no document open")?;
    let id = state.active_layer.ok_or("no active layer")?;
    if !state.doc.sprite.as_ref().is_some_and(|sprite| sprite.tilemap_descriptor(id).is_some()) {
        return Err("layer is not a retained tilemap".into());
    }
    check_tilemap_parent(&state.doc, id).map_err(|e| e.to_string())
}
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "sprite".into(), msg: message.into() }
}
fn integer(p: &Value, key: &str, max: u64) -> Result<u64> {
    p.get(key).and_then(Value::as_u64).filter(|n| *n <= max).ok_or_else(|| bad(format!("`{key}` must be an integer from 0 to {max}")))
}
fn mutate(s: &mut Session, label: &str, f: impl FnOnce(&mut Sprite) -> Result<()>) -> Result<Value> {
    s.edit(label, |doc, _| {
        doc.capture_sprite_frame().map_err(EngineError::Other)?;
        let sprite = doc.sprite.as_mut().ok_or_else(|| bad("no cel animation"))?;
        f(sprite)?;
        sprite.validate().map_err(EngineError::Other)?;
        if let Some(t) = &mut doc.timeline {
            t.duration = sprite.frames.len();
            t.current = sprite.current;
            t.work_end = t.work_end.min(t.duration).max(1);
        }
        doc.restore_sprite_frame().map_err(EngineError::Other)?;
        Ok(())
    })?;
    info(s)
}

fn info(s: &Session) -> Result<Value> {
    let sprite = s.active().and_then(|d| d.doc.sprite.as_ref()).ok_or_else(|| bad("no cel animation"))?;
    Ok(json!({"frames":sprite.frames.len(),"current":sprite.current,"indexed":sprite.indexed,
        "durationMs":sprite.duration_ms(),"durationsMs":sprite.frames.iter().map(|f|f.duration_ms).collect::<Vec<_>>(),
        "tags":sprite.tags,"pixelAspect":sprite.pixel_aspect,"transparentIndex":sprite.transparent_index,
        "palette":sprite.frames.get(sprite.current).map(|f|f.palette.as_ref())}))
}

fn duration(s: &mut Session, p: &Value) -> Result<Value> {
    let duration = integer(p, "durationMs", 65535)? as u16;
    mutate(s, "Frame Delay", |sprite| {
        let frame = p.get("frame").map(|_| integer(p, "frame", 9999).map(|n| n as usize)).transpose()?.unwrap_or(sprite.current);
        Arc::make_mut(sprite.frames.get_mut(frame).ok_or_else(|| bad("frame is out of range"))?).duration_ms = duration;
        Ok(())
    })
}

fn palette(s: &mut Session, p: &Value) -> Result<Value> {
    let index = integer(p, "index", 255)? as usize;
    let rgba = p.get("rgba").and_then(Value::as_array).filter(|a| a.len() == 4).ok_or_else(|| bad("need rgba:[r,g,b,a] with 4 bytes"))?;
    let mut color = [0; 4];
    for (out, value) in color.iter_mut().zip(rgba) {
        *out = value.as_u64().filter(|n| *n <= 255).ok_or_else(|| bad("palette RGBA components must be bytes"))? as u8;
    }
    let name = p
        .get("name")
        .map(|v| v.as_str().filter(|s| s.len() <= 65535).map(str::to_owned).ok_or_else(|| bad("palette name must be UTF-8 text up to 65535 bytes")))
        .transpose()?;
    mutate(s, "Palette Entry", |sprite| {
        let frame = Arc::make_mut(sprite.frames.get_mut(sprite.current).ok_or_else(|| bad("frame is missing"))?);
        let entries = Arc::make_mut(&mut frame.palette);
        if index >= entries.len() {
            entries.resize_with(index + 1, PaletteEntry::default);
        }
        let entry = entries.get_mut(index).ok_or_else(|| bad("palette entry is missing"))?;
        entry.rgba = color;
        if let Some(name) = name {
            entry.name = name;
        }
        if sprite.indexed {
            for (id, cel) in &mut frame.cels {
                let background = sprite.layers.iter().find(|l| l.id == *id).is_some_and(|l| l.flags & 8 != 0);
                let cel = Arc::make_mut(cel);
                if let Some(indices) = &cel.indices {
                    cel.pixels =
                        photocraft_doc::sprite::pixels::render_indices(indices, cel.rect, &frame.palette, (!background).then_some(sprite.transparent_index))
                            .map_err(EngineError::Other)?;
                }
            }
        }
        Ok(())
    })
}

fn cel(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = LayerId(integer(p, "layer", u64::MAX)?);
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    crate::commands::check_all_unlocked(doc, layer)?;
    let position_locked = doc.effective_locks(layer).position;
    mutate(s, "Cel Properties", |sprite| {
        let at = p.get("frame").map(|_| integer(p, "frame", 9999).map(|n| n as usize)).transpose()?.unwrap_or(sprite.current);
        let frame = Arc::make_mut(sprite.frames.get_mut(at).ok_or_else(|| bad("frame is out of range"))?);
        let cel = Arc::make_mut(frame.cels.get_mut(&layer).ok_or_else(|| bad("layer has no cel in this frame"))?);
        if let Some(opacity) = p.get("opacity") {
            cel.opacity = opacity.as_u64().filter(|n| *n <= 255).ok_or_else(|| bad("opacity must be a byte"))? as u8;
        }
        let z = p
            .get("zIndex")
            .map(|z| i16::try_from(z.as_i64().ok_or_else(|| bad("zIndex must be signed 16-bit"))?).map_err(|_| bad("zIndex must be signed 16-bit")))
            .transpose()?
            .unwrap_or(cel.z_index);
        let position = |key, default| {
            p.get(key)
                .map(|v| v.as_i64().and_then(|n| i16::try_from(n).ok()).map(i32::from).ok_or_else(|| bad(format!("{key} must be signed 16-bit"))))
                .transpose()
                .map(|v| v.unwrap_or(default))
        };
        let (x, y) = (position("x", cel.rect.x0)?, position("y", cel.rect.y0)?);
        let (dx, dy) =
            (x.checked_sub(cel.rect.x0).ok_or_else(|| bad("cel offset overflow"))?, y.checked_sub(cel.rect.y0).ok_or_else(|| bad("cel offset overflow"))?);
        if position_locked && (dx != 0 || dy != 0 || z != cel.z_index) {
            return Err(bad("the cel's layer or parent group is position-locked"));
        }
        cel.z_index = z;
        if dx != 0 || dy != 0 {
            cel.pixels = cel.pixels.translated(dx, dy, cel.rect);
            cel.indices = cel.indices.as_ref().map(|s| s.translated(dx, dy, cel.rect));
            cel.rect = cel.rect.translate(dx, dy);
            if let Some(bounds) = &mut cel.precise {
                for (i, d) in [(0, dx), (1, dy)] {
                    let v = bounds.get_mut(i).ok_or_else(|| bad("precise bounds missing"))?;
                    *v = i64::from(*v)
                        .checked_add(i64::from(d) * 65536)
                        .and_then(|n| i32::try_from(n).ok())
                        .ok_or_else(|| bad("precise cel offset overflow"))?;
                }
            }
        }
        Ok(())
    })
}

fn unlink(s: &mut Session, p: &Value) -> Result<Value> {
    let id = LayerId(integer(p, "layer", u64::MAX)?);
    crate::commands::check_all_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    mutate(s, "Unlink Cel", |sprite| {
        let frame = Arc::make_mut(sprite.frames.get_mut(sprite.current).ok_or_else(|| bad("frame is missing"))?);
        Arc::make_mut(frame.cels.get_mut(&id).ok_or_else(|| bad("layer has no cel"))?).linked_frame = None;
        Ok(())
    })
}

fn tags(s: &mut Session, p: &Value) -> Result<Value> {
    let value = p.get("tags").ok_or_else(|| bad("need tags:[{name,from,to,direction,repeat,color}]"))?;
    if !value.as_array().is_some_and(|v| v.len() <= 65535) {
        return Err(bad("tags must be an array of at most 65535 entries"));
    }
    let tags: Vec<SpriteTag> = serde_json::from_value(value.clone()).map_err(|e| bad(e.to_string()))?;
    if tags.len() > 65535 {
        return Err(bad("too many animation tags"));
    }
    mutate(s, "Animation Tags", |sprite| {
        if sprite.frames.iter().any(|f| f.metadata.iter().any(|c| c.target == photocraft_doc::sprite::MetadataTarget::Tags)) {
            return Err(bad("tags have retained user properties; edit a copy or retain the existing tag order"));
        }
        sprite.tags = tags;
        Ok(())
    })
}

fn duplicate(s: &mut Session) -> Result<Value> {
    mutate(s, "Duplicate Cel Frame", |sprite| {
        if sprite.frames.len() >= photocraft_doc::sprite::MAX_FRAMES {
            return Err(bad("frame limit reached"));
        }
        if sprite.frames.iter().any(|f| f.metadata.iter().any(|c| c.kind == 0x2022)) {
            return Err(bad("slices have frame keys; remove slices from a copy before inserting frames"));
        }
        let at = sprite.current + 1;
        let mut copied = sprite.frames.get(sprite.current).ok_or_else(|| bad("selected frame is missing"))?.as_ref().clone();
        copied.metadata.retain(|c| matches!(c.target, photocraft_doc::sprite::MetadataTarget::Cel(_)));
        // A duplicate is initially independent; sharing Arc tiles gives COW pixel storage.
        for cel in copied.cels.values_mut() {
            Arc::make_mut(cel).linked_frame = None;
        }
        for frame in &mut sprite.frames {
            for cel in Arc::make_mut(frame).cels.values_mut() {
                if let Some(link) = &mut Arc::make_mut(cel).linked_frame
                    && *link >= at
                {
                    *link += 1;
                }
            }
        }
        sprite.frames.insert(at, Arc::new(copied));
        for tag in &mut sprite.tags {
            if tag.from >= at {
                tag.from += 1;
            }
            if tag.to >= at {
                tag.to += 1;
            }
        }
        sprite.current = at;
        Ok(())
    })
}

fn remove_frame(s: &mut Session) -> Result<Value> {
    mutate(s, "Delete Cel Frame", |sprite| {
        use photocraft_doc::sprite::MetadataTarget;
        if sprite.frames.len() <= 1 {
            return Err(bad("a sprite must keep at least one frame"));
        }
        if sprite.frames.iter().any(|f| f.metadata.iter().any(|c| c.kind == 0x2022 || c.target == MetadataTarget::Tags)) {
            return Err(bad("slice keys or tag properties depend on frame ranges; remove that metadata from a copy before deleting frames"));
        }
        let at = sprite.current;
        let removed = sprite.frames.remove(at);
        for frame in &mut sprite.frames {
            for cel in Arc::make_mut(frame).cels.values_mut() {
                let cel = Arc::make_mut(cel);
                cel.linked_frame = match cel.linked_frame {
                    Some(link) if link == at => None,
                    Some(link) if link > at => Some(link - 1),
                    other => other,
                };
            }
        }
        if at == 0 {
            let first = Arc::make_mut(sprite.frames.first_mut().ok_or_else(|| bad("sprite frame is missing"))?);
            let mut global: Vec<_> = removed
                .metadata
                .iter()
                .filter(|c| matches!(c.target, MetadataTarget::Sprite | MetadataTarget::Layer(_) | MetadataTarget::Palette | MetadataTarget::Frame))
                .cloned()
                .collect();
            global.append(&mut first.metadata);
            first.metadata = global;
        }
        sprite.tags.retain_mut(|tag| {
            if tag.from == at && tag.to == at {
                return false;
            }
            if tag.from > at {
                tag.from -= 1
            }
            if tag.to >= at {
                tag.to = tag.to.saturating_sub(1)
            }
            true
        });
        sprite.current = at.min(sprite.frames.len() - 1);
        Ok(())
    })
}

fn detach(s: &mut Session) -> Result<Value> {
    s.edit("Keep Selected Frame", |doc, _| {
        doc.sprite = None;
        doc.timeline = None;
        Ok(())
    })?;
    Ok(json!({"sprite":null,"keptSelectedFrame":true}))
}

fn rasterize_tilemap(s: &mut Session, p: &Value) -> Result<Value> {
    let id = LayerId(integer(p, "layer", u64::MAX)?);
    s.edit("Rasterize Tilemap", |doc, _| {
        // Its own lock protects the retained display cache and is explicitly removed by
        // rasterization. A locked ancestor still protects the contents of that group.
        check_tilemap_parent(doc, id)?;
        let sprite = doc.sprite.as_mut().ok_or_else(|| bad("no cel animation"))?;
        if sprite.tilemap_descriptor(id).is_none() {
            return Err(bad("layer is not a retained tilemap"));
        }
        for frame in &mut sprite.frames {
            Arc::make_mut(frame).metadata.retain(|m| {
                !((m.kind == 0x2004 && m.target == photocraft_doc::sprite::MetadataTarget::Layer(id))
                    || (m.kind == 0x2005 && m.target == photocraft_doc::sprite::MetadataTarget::Cel(id)))
            });
        }
        if let Some(layer) = doc.layer_mut(id) {
            layer.locks.all = false;
        }
        Ok(())
    })?;
    info(s)
}

macro_rules! spec {
    ($id:literal,$label:literal,$params:literal,$run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled, journal: true, run: $run }
    };
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "sprite.playback",
            label: "Sprite Playback",
            menu: &[],
            shortcut: None,
            params: "{elapsedMs:u64,tag?:index} → {frame,finished}",
            enabled,
            journal: false,
            run: |s, p| {
                let elapsed = integer(p, "elapsedMs", u64::MAX)?;
                let tag = p.get("tag").filter(|v| !v.is_null()).map(|_| integer(p, "tag", 65535).map(|n| n as usize)).transpose()?;
                let sprite = s.active().and_then(|d| d.doc.sprite.as_ref()).ok_or_else(|| bad("no cel animation"))?;
                let (frame, finished) = sprite.playback_frame(elapsed, tag).map_err(EngineError::Other)?;
                Ok(json!({"frame":frame,"finished":finished}))
            },
        },
        CommandSpec {
            id: "sprite.info",
            label: "Sprite Info",
            menu: &[],
            shortcut: None,
            params: "{} → {frames,current,durationsMs,palette,tags}",
            enabled,
            journal: false,
            run: |s, _| info(s),
        },
        spec!("sprite.setDuration", "Frame Delay", "{durationMs:0..65535,frame?:index}", duration),
        spec!("sprite.setPaletteEntry", "Palette Entry", "{index:0..255,rgba:[r,g,b,a],name?:string}", palette),
        CommandSpec {
            enabled: cel_enabled,
            ..spec!("sprite.setCel", "Cel Properties", "{\"layer\":id,frame?:index,x?:i16,y?:i16,opacity?:0..255,zIndex?:i16}", cel)
        },
        CommandSpec { enabled: cel_enabled, ..spec!("sprite.unlinkCel", "Unlink Cel", "{\"layer\":id}", unlink) },
        spec!("sprite.setTags", "Animation Tags", "{tags:[{name,from,to,direction:0..3,repeat:0..65535,color:[r,g,b]}]}", tags),
        spec!("sprite.duplicateFrame", "Duplicate Cel Frame", "{}", |s, _| duplicate(s)),
        spec!("sprite.deleteFrame", "Delete Cel Frame", "{}", |s, _| remove_frame(s)),
        spec!("sprite.detach", "Keep Selected Frame", "{} → {sprite:null,keptSelectedFrame:true}", |s, _| detach(s)),
        CommandSpec {
            enabled: tilemap_enabled,
            ..spec!("sprite.rasterizeTilemap", "Rasterize Tilemap", "{\"layer\":id} → editable pixel cels (explicitly replaces tile ids)", rasterize_tilemap)
        },
    ]
}

#[cfg(test)]
mod tests;
