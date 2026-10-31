//! Experimental, local-only editable mask brush stacks for the UV workspace.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_color::BlendMode;
use photocraft_doc::{
    LayerId,
    mask_paint::{EffectKind, Paint, Stack},
};
use photocraft_raster::Surface;
use serde_json::{Value, json};

fn bad(message: &str) -> EngineError {
    EngineError::Other(message.into())
}
fn mask_blend(value: &str) -> Option<BlendMode> {
    let mode = match value {
        "normal" => BlendMode::Normal,
        "darken" => BlendMode::Darken,
        "multiply" => BlendMode::Multiply,
        "lighten" => BlendMode::Lighten,
        "screen" => BlendMode::Screen,
        "linear_dodge" => BlendMode::LinearDodge,
        "subtract" => BlendMode::Subtract,
        "difference" => BlendMode::Difference,
        _ => return None,
    };
    photocraft_doc::mask_paint::is_mask_blend_mode(mode).then_some(mode)
}
fn blend_param(p: &Value) -> Result<BlendMode> {
    p.get("blend")
        .map(|value| value.as_str().and_then(mask_blend).ok_or_else(|| bad("Invalid grayscale mask blend mode")))
        .transpose()
        .map(|mode| mode.unwrap_or(BlendMode::Normal))
}
fn radius_param(p: &Value, default: u8) -> Result<u8> {
    p.get("radius")
        .map(|value| {
            value
                .as_u64()
                .and_then(|v| u8::try_from(v).ok())
                .filter(|v| (1..=64).contains(v))
                .ok_or_else(|| bad("Blur radius must be an integer from 1 to 64 pixels"))
        })
        .transpose()
        .map(|radius| radius.unwrap_or(default))
}
fn target(s: &Session, p: &Value) -> Result<LayerId> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = p.get("layer").and_then(Value::as_u64).map(LayerId).or(st.active_layer).ok_or_else(|| bad("Select a masked layer"))?;
    let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if l.mask.is_none() {
        return Err(bad("Layer has no mask"));
    }
    if st.doc.effective_locks(id).all {
        return Err(bad("Layer is locked"));
    }
    Ok(id)
}
fn index(p: &Value) -> Result<usize> {
    p.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad("Invalid paint layer index"))
}
fn unit_param(p: &Value, key: &str, default: f32) -> Result<f32> {
    p.get(key)
        .map(|v| v.as_f64().filter(|n| n.is_finite() && (0.0..=1.0).contains(n)).map(|n| n as f32).ok_or_else(|| bad("Effect values must be between 0 and 1")))
        .transpose()
        .map(|v| v.unwrap_or(default))
}
fn gamma_param(p: &Value) -> Result<f32> {
    p.get("gamma")
        .map(|v| v.as_f64().filter(|n| n.is_finite() && (0.01..=100.0).contains(n)).map(|n| n as f32).ok_or_else(|| bad("Gamma must be between 0.01 and 100")))
        .transpose()
        .map(|v| v.unwrap_or(1.0))
}
fn rotation_param(p: &Value) -> Result<f32> {
    p.get("rotation")
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite() && (-36_000.0..=36_000.0).contains(n))
                .map(|n| n as f32)
                .ok_or_else(|| bad("Fill rotation must be between -36000 and 36000 degrees"))
        })
        .transpose()
        .map(|v| v.unwrap_or(0.0))
}
fn bool_param(p: &Value, key: &str, default: bool) -> Result<bool> {
    p.get(key)
        .map(|value| value.as_bool().ok_or_else(|| bad("Fill repeat and mirror options must be boolean")))
        .transpose()
        .map(|value| value.unwrap_or(default))
}
fn validate_effect(effect: &EffectKind) -> Result<()> {
    match effect {
        EffectKind::Paint => Ok(()),
        EffectKind::Fill { value, rotation, .. } if value.is_finite() && (0.0..=1.0).contains(value) && rotation.is_finite() => Ok(()),
        EffectKind::Fill { .. } => Err(bad("Fill value must be between 0 and 1 and rotation must be finite")),
        EffectKind::Levels { input_black, gamma, input_white, output_black, output_white }
            if input_black.is_finite()
                && gamma.is_finite()
                && input_white.is_finite()
                && output_black.is_finite()
                && output_white.is_finite()
                && (0.0..=1.0).contains(input_black)
                && (0.01..=100.0).contains(gamma)
                && (0.0..=1.0).contains(input_white)
                && (0.0..=1.0).contains(output_black)
                && (0.0..=1.0).contains(output_white)
                && input_black < input_white
                && output_black <= output_white =>
        {
            Ok(())
        }
        EffectKind::Levels { .. } => Err(bad("Invalid Levels range")),
        EffectKind::Blur { radius } if (1..=64).contains(radius) => Ok(()),
        EffectKind::Blur { .. } => Err(bad("Blur radius must be between 1 and 64 pixels")),
    }
}
fn validate_blur_budget(layers: &[Paint]) -> Result<()> {
    let total = layers
        .iter()
        .filter_map(|layer| match &layer.kind {
            EffectKind::Blur { radius } => Some(u32::from(*radius)),
            _ => None,
        })
        .fold(0u32, u32::saturating_add);
    if total > u32::from(photocraft_doc::mask_paint::MAX_BLUR_RADIUS_SUM) {
        return Err(bad("The combined Blur radius in a mask stack cannot exceed 512 pixels"));
    }
    Ok(())
}
fn has_any(p: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|key| p.get(key).is_some())
}
fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p)?;
    let canvas = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let from_selection = p.get("from_selection").and_then(Value::as_bool).unwrap_or(false);
    let selection = if from_selection { Some(s.active().and_then(|st| st.doc.selection.clone()).ok_or_else(|| bad("Make a selection first"))?) } else { None };
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("paint").to_owned();
    let blend_supplied = p.get("blend").is_some();
    let radius_supplied = p.get("radius").is_some();
    let blend = blend_param(p)?;
    let radius = radius_param(p, 8)?;
    let default_name = match kind.as_str() {
        "fill" => "Fill",
        "levels" => "Levels",
        "blur" => "Blur",
        _ => "Paint",
    };
    let name = p.get("name").and_then(Value::as_str).unwrap_or(default_name).chars().take(128).collect::<String>();
    let result = s.edit("Add mask Paint layer", |doc, active| {
        let canvas = doc.bounds();
        let mask = doc.layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or_else(|| bad("Layer has no mask"))?;
        let format = photocraft_color::PixelFormat::RGBA8.with_sample(mask.surface.format().sample);
        let base = mask.surface.clone();
        let stack = mask.paint.get_or_insert_with(|| Stack { base, layers: Vec::new(), active: 0 });
        if stack.layers.len() >= 256 {
            return Err(bad("Mask supports up to 256 Paint layers"));
        }
        let surface = Surface::new(format);
        let effect = match kind.as_str() {
            "paint" => EffectKind::Paint,
            "fill" => EffectKind::Fill {
                value: unit_param(p, "value", 1.0)?,
                rotation: rotation_param(p)?,
                repeat_u: bool_param(p, "repeat_u", true)?,
                repeat_v: bool_param(p, "repeat_v", true)?,
                mirror_u: bool_param(p, "mirror_u", false)?,
                mirror_v: bool_param(p, "mirror_v", false)?,
            },
            "levels" => EffectKind::Levels {
                input_black: unit_param(p, "input_black", 0.0)?,
                gamma: gamma_param(p)?,
                input_white: unit_param(p, "input_white", 1.0)?,
                output_black: unit_param(p, "output_black", 0.0)?,
                output_white: unit_param(p, "output_white", 1.0)?,
            },
            "blur" => EffectKind::Blur { radius },
            _ => return Err(bad("Kind must be paint, fill, levels, or blur")),
        };
        validate_effect(&effect)?;
        if blend_supplied && !matches!(&effect, EffectKind::Paint | EffectKind::Fill { .. } | EffectKind::Blur { .. }) {
            return Err(bad("Blend mode applies only to Paint, Fill, and Blur mask layers"));
        }
        if radius_supplied && !matches!(&effect, EffectKind::Blur { .. }) {
            return Err(bad("Radius applies only to Blur mask layers"));
        }
        match &effect {
            EffectKind::Paint
                if has_any(
                    p,
                    &[
                        "value",
                        "rotation",
                        "repeat_u",
                        "repeat_v",
                        "mirror_u",
                        "mirror_v",
                        "input_black",
                        "gamma",
                        "input_white",
                        "output_black",
                        "output_white",
                    ],
                ) =>
            {
                return Err(bad("Effect settings do not apply to Paint layers"));
            }
            EffectKind::Fill { .. } if has_any(p, &["input_black", "gamma", "input_white", "output_black", "output_white"]) => {
                return Err(bad("Levels settings do not apply to Fill layers"));
            }
            EffectKind::Levels { .. } if has_any(p, &["value", "rotation", "repeat_u", "repeat_v", "mirror_u", "mirror_v"]) => {
                return Err(bad("Fill settings do not apply to Levels layers"));
            }
            _ => {}
        }
        let (fill_map, fill_map_size, fill_bounds) = if matches!(&effect, EffectKind::Fill { .. }) {
            if let Some(selection) = &selection {
                let bounds = selection.content_bounds().intersect(&canvas);
                if bounds.is_empty() {
                    return Err(bad("The selection is empty"));
                }
                let width = bounds.width();
                let height = bounds.height();
                if u64::from(width) * u64::from(height) > 64_000_000 {
                    return Err(bad("Fill selections must contain at most 64 million pixels"));
                }
                let mut map = Surface::with_default(format, &[0.0, 0.0, 0.0, 1.0]);
                for y0 in (bounds.y0..bounds.y1).step_by(256) {
                    let y1 = y0.saturating_add(256).min(bounds.y1);
                    let pixel_count = (width as usize)
                        .checked_mul((y1 - y0) as usize)
                        .and_then(|count| count.checked_mul(4))
                        .ok_or_else(|| bad("Fill selection is too large"))?;
                    let mut pixels = Vec::new();
                    pixels.try_reserve_exact(pixel_count).map_err(|_| bad("Could not allocate Fill selection band"))?;
                    for y in y0..y1 {
                        for x in bounds.x0..bounds.x1 {
                            let value = selection.sample_channel(x, y, 0).clamp(0.0, 1.0);
                            pixels.extend_from_slice(&[value, value, value, 1.0]);
                        }
                    }
                    let band = photocraft_geom::Rect::new(0, y0 - bounds.y0, width as i32, y1 - bounds.y0);
                    map.try_write_region(band, &pixels).map_err(EngineError::from)?;
                }
                (Some(map), [width, height], Some([bounds.x0, bounds.y0, bounds.x1, bounds.y1]))
            } else {
                (None, [0, 0], Some([canvas.x0, canvas.y0, canvas.x1, canvas.y1]))
            }
        } else {
            (None, [0, 0], None)
        };
        stack.layers.push(Paint { name, visible: true, opacity: 1.0, blend, surface, fill_map, fill_map_size, fill_bounds, kind: effect });
        validate_blur_budget(&stack.layers)?;
        stack.active = stack.layers.len().saturating_sub(1);
        *active = Some(id);
        Ok(json!({"index":stack.active}))
    })?;
    // In Gray mask view, the displayed image is the evaluated mask stack itself.
    // Adding an effect can therefore change pixels even when the ordinary layer
    // compositor reports empty damage (for example, an empty layer with a Fill).
    // Invalidate the whole canvas so the UV mask preview cannot retain stale pixels.
    if let Some(state) = s.active_mut() {
        state.last_damage = Some(canvas);
    }
    Ok(result)
}
fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p)?;
    let index = index(p)?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let mask = std::sync::Arc::make_mut(&mut st.doc).layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or_else(|| bad("Layer has no mask"))?;
    let stack = mask.paint.as_mut().ok_or_else(|| bad("Mask has no Paint layers"))?;
    if index >= stack.layers.len() {
        return Err(bad("Paint layer is out of range"));
    }
    stack.active = index;
    st.active_layer = Some(id);
    Ok(Value::Null)
}
fn modify(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p)?;
    let index = index(p)?;
    let opacity = p
        .get("opacity")
        .map(|v| v.as_f64().filter(|n| n.is_finite() && (0.0..=1.0).contains(n)).ok_or_else(|| bad("Opacity must be between 0 and 1")))
        .transpose()?;
    let visible = p.get("visible").map(|v| v.as_bool().ok_or_else(|| bad("Visibility must be boolean"))).transpose()?;
    let destination = p.get("to").map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad("Invalid destination"))).transpose()?;
    let remove = p.get("remove").and_then(Value::as_bool).unwrap_or(false);
    let clear_fill_map = p.get("clear_fill_map").map(|v| v.as_bool().ok_or_else(|| bad("clear_fill_map must be boolean"))).transpose()?.unwrap_or(false);
    let fill_value = p.get("value").map(|_| unit_param(p, "value", 0.0)).transpose()?;
    let rotation = p.get("rotation").map(|_| rotation_param(p)).transpose()?;
    let repeat_u = p.get("repeat_u").map(|_| bool_param(p, "repeat_u", true)).transpose()?;
    let repeat_v = p.get("repeat_v").map(|_| bool_param(p, "repeat_v", true)).transpose()?;
    let mirror_u = p.get("mirror_u").map(|_| bool_param(p, "mirror_u", false)).transpose()?;
    let mirror_v = p.get("mirror_v").map(|_| bool_param(p, "mirror_v", false)).transpose()?;
    let input_black = p.get("input_black").map(|_| unit_param(p, "input_black", 0.0)).transpose()?;
    let input_white = p.get("input_white").map(|_| unit_param(p, "input_white", 1.0)).transpose()?;
    let output_black = p.get("output_black").map(|_| unit_param(p, "output_black", 0.0)).transpose()?;
    let output_white = p.get("output_white").map(|_| unit_param(p, "output_white", 1.0)).transpose()?;
    let gamma = p.get("gamma").map(|_| gamma_param(p)).transpose()?;
    let blend = p.get("blend").map(|_| blend_param(p)).transpose()?;
    let radius = p.get("radius").map(|_| radius_param(p, 8)).transpose()?;
    let bounds = p
        .get("bounds")
        .and_then(Value::as_array)
        .map(|v| {
            if v.len() != 4 {
                return Err(bad("Fill bounds require four coordinates"));
            }
            let mut out = [0i32; 4];
            for (i, x) in v.iter().enumerate() {
                out[i] = i32::try_from(x.as_i64().ok_or_else(|| bad("Invalid fill bounds"))?).map_err(|_| bad("Invalid fill bounds"))?;
            }
            if out[2] <= out[0] || out[3] <= out[1] {
                return Err(bad("Fill bounds must have positive size"));
            }
            Ok(out)
        })
        .transpose()?;
    let effect_changed = fill_value.is_some()
        || rotation.is_some()
        || repeat_u.is_some()
        || repeat_v.is_some()
        || mirror_u.is_some()
        || mirror_v.is_some()
        || input_black.is_some()
        || input_white.is_some()
        || output_black.is_some()
        || output_white.is_some()
        || gamma.is_some()
        || bounds.is_some()
        || clear_fill_map;
    let effect_changed = effect_changed || blend.is_some() || radius.is_some();
    let mut damage = photocraft_geom::Rect::EMPTY;
    let result = s.edit("Edit mask Paint layer", |doc, _| {
        let canvas = doc.bounds();
        let mask = doc.layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or_else(|| bad("Layer has no mask"))?;
        let stack = mask.paint.as_mut().ok_or_else(|| bad("Mask has no Paint layers"))?;
        if destination.is_some_and(|to| to >= stack.layers.len()) {
            return Err(bad("Destination is out of range"));
        }
        if opacity.is_some() || visible.is_some() || remove || destination.is_some() || effect_changed {
            // A Paint edit can change pixels beyond its own surface bounds when Blur layers
            // follow it. Use the old stack here so hiding/removing/moving the Paint reports the
            // pixels its former downstream Blurs used to influence as well.
            let blur_reach = if destination.is_none() && matches!(stack.layers.get(index).map(|paint| &paint.kind), Some(EffectKind::Paint)) {
                photocraft_compose::mask_paint::downstream_blur_reach(stack, index)
            } else {
                0
            };
            let affected = if destination.is_some() { stack.layers.iter().collect::<Vec<_>>() } else { stack.layers.get(index).into_iter().collect() };
            for paint in affected {
                let bounds = if !matches!(&paint.kind, EffectKind::Paint) || paint.surface.default_pixel().get(3).is_some_and(|a| *a != 0.0) {
                    canvas
                } else {
                    paint.surface.content_bounds()
                };
                damage = damage.union(&bounds);
            }
            if !damage.is_empty() {
                let feather = (mask.feather.max(0.0) * 3.0).ceil().min(100_000.0) as i32;
                damage = damage.inflate(feather.saturating_add(blur_reach)).intersect(&canvas);
            }
        }
        let paint = stack.layers.get_mut(index).ok_or_else(|| bad("Paint layer is out of range"))?;
        if let Some(value) = opacity {
            paint.opacity = value as f32;
        }
        if let Some(value) = visible {
            paint.visible = value;
        }
        if let Some(value) = blend {
            paint.blend = value;
        }
        if let Some(name) = p.get("name").and_then(Value::as_str) {
            paint.name = name.chars().take(128).collect();
        }
        match &mut paint.kind {
            EffectKind::Fill {
                value,
                rotation: fill_rotation,
                repeat_u: fill_repeat_u,
                repeat_v: fill_repeat_v,
                mirror_u: fill_mirror_u,
                mirror_v: fill_mirror_v,
            } => {
                if input_black.is_some() || gamma.is_some() || input_white.is_some() || output_black.is_some() || output_white.is_some() || radius.is_some() {
                    return Err(bad("Levels and Blur settings do not apply to Fill layers"));
                }
                if let Some(next) = fill_value {
                    *value = next;
                }
                if let Some(next) = rotation {
                    *fill_rotation = next;
                }
                if let Some(next) = repeat_u {
                    *fill_repeat_u = next;
                }
                if let Some(next) = repeat_v {
                    *fill_repeat_v = next;
                }
                if let Some(next) = mirror_u {
                    *fill_mirror_u = next;
                }
                if let Some(next) = mirror_v {
                    *fill_mirror_v = next;
                }
                if let Some(next) = bounds {
                    paint.fill_bounds = Some(next);
                }
                if clear_fill_map {
                    paint.fill_map = None;
                    paint.fill_map_size = [0, 0];
                }
            }
            EffectKind::Levels { input_black: ib, gamma: g, input_white: iw, output_black: ob, output_white: ow } => {
                if blend.is_some() {
                    return Err(bad("Blend mode does not apply to Levels mask layers"));
                }
                if fill_value.is_some()
                    || rotation.is_some()
                    || repeat_u.is_some()
                    || repeat_v.is_some()
                    || mirror_u.is_some()
                    || mirror_v.is_some()
                    || bounds.is_some()
                    || clear_fill_map
                    || radius.is_some()
                {
                    return Err(bad("Fill and Blur settings do not apply to Levels layers"));
                }
                if let Some(next) = input_black {
                    *ib = next;
                }
                if let Some(next) = gamma {
                    *g = next;
                }
                if let Some(next) = input_white {
                    *iw = next;
                }
                if let Some(next) = output_black {
                    *ob = next;
                }
                if let Some(next) = output_white {
                    *ow = next;
                }
            }
            EffectKind::Paint => {
                if fill_value.is_some()
                    || rotation.is_some()
                    || repeat_u.is_some()
                    || repeat_v.is_some()
                    || mirror_u.is_some()
                    || mirror_v.is_some()
                    || bounds.is_some()
                    || input_black.is_some()
                    || gamma.is_some()
                    || input_white.is_some()
                    || output_black.is_some()
                    || output_white.is_some()
                    || clear_fill_map
                    || radius.is_some()
                {
                    return Err(bad("Effect settings do not apply to Paint layers"));
                }
            }
            EffectKind::Blur { radius: current } => {
                if fill_value.is_some()
                    || rotation.is_some()
                    || repeat_u.is_some()
                    || repeat_v.is_some()
                    || mirror_u.is_some()
                    || mirror_v.is_some()
                    || bounds.is_some()
                    || input_black.is_some()
                    || gamma.is_some()
                    || input_white.is_some()
                    || output_black.is_some()
                    || output_white.is_some()
                    || clear_fill_map
                {
                    return Err(bad("Effect settings do not apply to Blur layers"));
                }
                if let Some(next) = radius {
                    *current = next;
                }
            }
        }
        validate_effect(&paint.kind)?;
        if remove {
            stack.layers.remove(index);
            stack.active = if stack.layers.is_empty() {
                0
            } else if index < stack.active {
                stack.active - 1
            } else {
                stack.active.min(stack.layers.len() - 1)
            };
        } else if let Some(to) = destination {
            let selected = stack.active;
            let paint = stack.layers.remove(index);
            stack.layers.insert(to, paint);
            stack.active = if selected == index {
                to
            } else if index < selected && to >= selected {
                selected.saturating_sub(1)
            } else if index > selected && to <= selected {
                selected.saturating_add(1)
            } else {
                selected
            };
        }
        if mask.paint.as_ref().is_some_and(|s| s.layers.is_empty()) {
            // No other paint remains: restore the original mask without a full blend pass.
            if let Some(stack) = &mask.paint {
                mask.surface = stack.base.clone();
            }
            mask.paint = None;
        }
        if let Some(stack) = &mask.paint {
            validate_blur_budget(&stack.layers)?;
        }
        Ok(Value::Null)
    })?;
    if let Some(st) = s.active_mut() {
        st.last_damage = Some(damage);
    }
    Ok(result)
}
fn fill_source(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p)?;
    let index = index(p)?;
    let asset_id = p.get("asset").and_then(Value::as_u64).ok_or_else(|| bad("Choose an image from the library"))?;
    let asset = s.uv.assets.iter().find(|a| a.id == asset_id).ok_or_else(|| bad("Image is no longer in the library"))?;
    let [width, height] = [asset.document.size.width, asset.document.size.height];
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 64_000_000 {
        return Err(bad("Fill images must contain at most 64 million pixels"));
    }
    let format = photocraft_color::PixelFormat::RGBA8;
    let mut map = Surface::with_default(format, &[0.0, 0.0, 0.0, 1.0]);
    let canvas = asset.document.bounds();
    let mut x = canvas.x0;
    while x < canvas.x1 {
        let slab = photocraft_geom::Rect::new(x, canvas.y0, x.saturating_add(4096).min(canvas.x1), canvas.y1);
        photocraft_compose::render_bands(&asset.document, slab, 256, |band| -> Result<()> {
            let rgba = band
                .px
                .iter()
                .flat_map(|pixel| {
                    let gray = (pixel[0] * 0.2126 + pixel[1] * 0.7152 + pixel[2] * 0.0722).clamp(0.0, 1.0);
                    [gray, gray, gray, 1.0]
                })
                .collect::<Vec<_>>();
            map.try_write_region(band.rect, &rgba).map_err(EngineError::from)?;
            Ok(())
        })?;
        x = slab.x1;
    }
    s.edit("Set Fill image", |doc, _| {
        let canvas = doc.bounds();
        let mask = doc.layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or_else(|| bad("Layer has no mask"))?;
        let stack = mask.paint.as_mut().ok_or_else(|| bad("Mask has no effect layers"))?;
        let fill = stack.layers.get_mut(index).ok_or_else(|| bad("Effect layer is out of range"))?;
        if !matches!(&fill.kind, EffectKind::Fill { .. }) {
            return Err(bad("Select a Fill layer first"));
        }
        fill.fill_map = Some(map);
        fill.fill_map_size = [width, height];
        if fill.fill_bounds.is_none() {
            fill.fill_bounds = Some([canvas.x0, canvas.y0, canvas.x1, canvas.y1]);
        }
        Ok(Value::Null)
    })
}
pub fn specs() -> Vec<CommandSpec> {
    [
        ("uv.maskPaint.add", "Add mask stack layer", r#"{"layer":id?,"name":string?,"kind":"paint"|"fill"|"levels"|"blur"?,"blend":"normal"|"darken"|"multiply"|"lighten"|"screen"|"linear_dodge"|"subtract"|"difference"?,"radius":1..64?,"value":number?,"rotation":degrees?,"from_selection":bool?,"repeat_u":bool?,"repeat_v":bool?,"mirror_u":bool?,"mirror_v":bool?,"input_black":number?,"gamma":number?,"input_white":number?,"output_black":number?,"output_white":number?}"#, add as fn(&mut Session, &Value) -> Result<Value>),
        ("uv.maskPaint.select", "Select mask Paint layer", r#"{"layer":id?,"index":int}"#, select),
        ("uv.maskPaint.fillSource", "Set Fill image", r#"{"layer":id?,"index":int,"asset":int}"#, fill_source),
        (
            "uv.maskPaint.modify",
            "Edit mask Paint layer",
            r#"{"layer":id?,"index":int,"visible":bool?,"opacity":number?,"name":string?,"to":int?,"remove":bool?,"blend":"normal"|"darken"|"multiply"|"lighten"|"screen"|"linear_dodge"|"subtract"|"difference"?,"radius":1..64?,"value":number?,"rotation":degrees?,"bounds":[i32,i32,i32,i32]?,"repeat_u":bool?,"repeat_v":bool?,"mirror_u":bool?,"mirror_v":bool?,"clear_fill_map":bool?,"input_black":number?,"gamma":number?,"input_white":number?,"output_black":number?,"output_white":number?}"#,
            modify,
        ),
    ]
    .into_iter()
    .map(|(id, label, params, run)| CommandSpec {
        id,
        label,
        params,
        run,
        menu: &[],
        shortcut: None,
        journal: id != "uv.maskPaint.select",
        enabled: |s| if s.uv.scene.is_some() { Ok(()) } else { Err("Import a model first".into()) },
    })
    .collect()
}
