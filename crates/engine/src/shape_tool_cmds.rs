//! Shape-tool pixel drawing.
use crate::commands::{CommandSpec, blend_from_str, has_paintable};
use crate::vector_cmds::{bad, parse_path, shape_path};
use crate::{Result, Session};
use photocraft_doc::LayerId;
use photocraft_geom::{Rect, TILE_SIZE};
use serde_json::{Value, json};

fn draw_pixels(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.drawPixels";
    let path = if p.get("kind").and_then(Value::as_str) == Some("path") {
        parse_path(p.get("path").ok_or_else(|| bad(CMD, "Missing path"))?).map_err(|e| bad(CMD, e))?
    } else {
        shape_path(p)?
    };
    photocraft_vector::flatten::validate_shape(&path, None).map_err(|e| bad(CMD, e))?;
    let opacity = match p.get("opacity") {
        None => 1.0,
        Some(v) => v.as_f64().filter(|v| (0.0..=100.0).contains(v)).ok_or_else(|| bad(CMD, "Opacity must be 0..100"))? as f32 / 100.0,
    };
    let mode = blend_from_str(string(p, "mode", "normal", CMD)?).ok_or_else(|| bad(CMD, "Unknown blend mode"))?;
    if let Some(target) = p.get("target") {
        let valid = target.as_str().is_some_and(|s| matches!(s, "pixels" | "mask" | "quickMask"))
            || target.get("channel").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).is_some();
        if !valid {
            return Err(bad(CMD, "Invalid paint target"));
        }
    }
    let aa = match p.get("antiAlias") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| bad(CMD, "antiAlias must be boolean"))?,
    };
    let color = match p.get("color") {
        None => s.tools.foreground,
        Some(v) => {
            let c = crate::vector_cmds::color(v).ok_or_else(|| bad(CMD, "Invalid color"))?;
            let rgb = c.to_rgb();
            [rgb[0], rgb[1], rgb[2], c.alpha]
        }
    };
    if color.iter().any(|c| !c.is_finite()) {
        return Err(bad(CMD, "Color must be finite"));
    }
    let active_layer = s.active().and_then(|st| st.active_layer);
    let restrict_target = if matches!(crate::channel_cmds::target_of(p), crate::channel_cmds::Target::Pixels) {
        p.get("layer").and_then(Value::as_u64).map(LayerId).filter(|target| Some(*target) != active_layer).zip(s.color_restrict)
    } else {
        None
    };
    let before = restrict_target.and_then(|_| s.active().map(|st| st.doc.clone()));
    let damage = s.edit("Draw Pixel Shape", |doc, active| {
        let raster = photocraft_vector::fill_rasterizer(&path, photocraft_vector::DEFAULT_TOLERANCE);
        let area = raster.pixel_bounds().unwrap_or(Rect::EMPTY).intersect(&doc.bounds());
        let selection = doc.selection.clone();
        let id = match p.get("layer") {
            Some(v) => Some(LayerId(v.as_u64().ok_or_else(|| bad(CMD, "Invalid layer id"))?)),
            None => *active,
        };
        if matches!(crate::channel_cmds::target_of(p), crate::channel_cmds::Target::Pixels | crate::channel_cmds::Target::Mask)
            && id.is_some_and(|id| doc.effective_locks(id).all)
        {
            return Err(bad(CMD, "Layer is locked"));
        }
        let (surface, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
        // Match the resampling memory policy, accounting for complete stored tiles.
        // A sparse document can have huge dimensions without allocating any pixels.
        if !area.is_empty() {
            let tiles_x = ((area.x1 - 1).div_euclid(TILE_SIZE) - area.x0.div_euclid(TILE_SIZE) + 1) as u64;
            let tiles_y = ((area.y1 - 1).div_euclid(TILE_SIZE) - area.y0.div_euclid(TILE_SIZE) + 1) as u64;
            let bytes = tiles_x
                .saturating_mul(tiles_y)
                .saturating_mul(TILE_SIZE as u64)
                .saturating_mul(TILE_SIZE as u64)
                .saturating_mul(surface.format().bytes_per_pixel() as u64);
            let limit = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 2 << 30 };
            if bytes > limit {
                return Err(bad(CMD, "Pixel shape exceeds drawing memory limit"));
            }
        }
        // Bounded tile buffers, including for very large documents.
        let step = TILE_SIZE as usize;
        for y in (area.y0..area.y1).step_by(step) {
            for x in (area.x0..area.x1).step_by(step) {
                let tile = Rect::new(x, y, x.saturating_add(TILE_SIZE).min(area.x1), y.saturating_add(TILE_SIZE).min(area.y1));
                let mut cov = raster.render(tile);
                let width = tile.width() as usize;
                for (i, c) in cov.iter_mut().enumerate() {
                    if !aa {
                        *c = f32::from(*c >= 0.5);
                    }
                    if let Some(sel) = &selection {
                        *c *= sel.sample_channel(tile.x0 + (i % width) as i32, tile.y0 + (i / width) as i32, 0);
                    }
                }
                crate::vector_cmds::paint_coverage(surface, tile, &cov, color, opacity, mode, lock);
            }
        }
        surface.prune();
        if let (Some((target, channel)), Some(before)) = (restrict_target, before.as_deref()) {
            crate::channel_cmds::restrict_to_color(before, doc, target, channel);
        }
        Ok(area)
    })?;
    Ok(json!({"damage": damage}))
}

fn string<'a>(p: &'a Value, key: &str, default: &'a str, cmd: &str) -> Result<&'a str> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v.as_str().ok_or_else(|| bad(cmd, format!("{key} must be a string"))),
    }
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "shape.drawPixels",
        label: "Draw Pixel Shape",
        menu: &[],
        shortcut: None,
        params: r##"{kind,rect/from/to/path: shape.create geometry,color: foreground,mode: blend mode,opacity: 0..100=100,antiAlias: bool=true,target: pixels|mask|quickMask|{channel:i},"layer":id?} (drawing bounds limited to 8 GiB of tiles on 64-bit, 2 GiB on 32-bit)"##,
        enabled: has_paintable,
        run: draw_pixels,
        journal: true,
    }]
}
