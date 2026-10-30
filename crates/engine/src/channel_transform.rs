//! Isolated geometric edits of a native colour component, mask or saved channel.
//! Colour components are scalar data: resampling them must never move the layer's alpha.

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba};
use serde_json::{Value, json};

use crate::channel_cmds::ChannelTarget;
use crate::{EngineError, Result, Session};

/// Frozen identity of the channel being transformed by an interactive frontend.
#[derive(Clone, Debug)]
pub struct Guard {
    pub layer: Option<LayerId>,
    pub target: Value,
}

pub const LOCKED: &str = "Commit or cancel the channel transform before changing its target or document pixels";
pub const HIDDEN: &str = "Show the selected channel using its eye before starting a transform";

/// The captured target stays locked until its background commit has ended too.
pub fn locked(s: &Session) -> bool {
    s.active().is_some_and(|st| st.channel_transform.is_some()) || s.channel_transform_running()
}

/// View commands remain available. Pixel/structural edits cannot invalidate the captured source.
pub(crate) fn refusal(s: &Session, id: &str, params: &Value, journal: bool) -> Option<&'static str> {
    if !locked(s) {
        return None;
    }
    (journal && id != "channel.setVisible" && !crate::warp_cmds::inline_edit(id, params)
        || id.starts_with("channel.target")
        || id.starts_with("layer.select")
        || (id.starts_with("timeline.") && id != "timeline.info")
        || matches!(id, "select.editInQuickMaskMode" | "edit.undo" | "edit.redo" | "history.undo" | "history.redo"))
    .then_some(LOCKED)
}

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "edit.transform".into(), msg: message.into() }
}

/// Geometric commands sharing scalar-channel resolution (mesh-editing commands are separate).
pub(crate) fn geometric(id: &str) -> bool {
    matches!(
        id,
        "edit.transform"
            | "edit.transform.warp"
            | "edit.transform.rotate180"
            | "edit.transform.rotate90Cw"
            | "edit.transform.rotate90Ccw"
            | "edit.transform.flipHorizontal"
            | "edit.transform.flipVertical"
    )
}

/// Resolve once, without selecting a channel (which would also change its eyes).
pub fn params(s: &Session, p: &Value) -> Result<Value> {
    let mut out = if p.is_null() { json!({}) } else { p.clone() };
    let map = out.as_object_mut().ok_or_else(|| bad("params must be an object"))?;
    if map.get("layer").is_some_and(Value::is_null) {
        map.remove("layer");
    }
    if let Some(layer) = crate::commands::explicit_layer(s, p)? {
        map.insert("layer".into(), json!(layer.0));
    }
    if let Some(target) = map.get("target") {
        // A Smart Filter mask can be painted and filtered, but geometric edits move its
        // smart object. Preserve that command behaviour even when the target is explicit.
        if target == "filterMask" {
            map.insert("target".into(), json!("pixels"));
            return Ok(out);
        }
        let valid = matches!(target.as_str(), Some("pixels" | "mask" | "quickMask"))
            || target.as_object().is_some_and(|o| {
                o.len() == 1 && (o.get("color").or_else(|| o.get("channel"))).is_some_and(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).is_some())
            });
        if !valid {
            return Err(bad("target must be pixels, mask, quickMask, {color:index} or {channel:index}"));
        }
        return Ok(out);
    }
    if map.contains_key("layer") || map.get("layers").is_some_and(|v| !v.is_null()) {
        map.insert("target".into(), json!("pixels"));
        return Ok(out);
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let target = match st.channel_view.target {
        ChannelTarget::Color(k) => json!({"color": k}),
        ChannelTarget::Alpha(i) => json!({"channel": i}),
        ChannelTarget::LayerMask => json!("mask"),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => json!("quickMask"),
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some_and(|v| !v.filter) => json!("mask"),
        _ => json!("pixels"),
    };
    map.insert("target".into(), target);
    Ok(out)
}

pub fn color_index(p: &Value) -> Result<Option<usize>> {
    p.get("target")
        .and_then(|t| t.get("color"))
        .map(|v| v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("invalid colour channel index")))
        .transpose()
}

/// Whether the channel's Channels-panel eye is currently on.
pub fn visible(s: &Session, p: &Value) -> Result<bool> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if let Some(k) = color_index(p)? {
        return Ok(st.channel_view.color_visible(k) && crate::mask_view_cmds::current(st).is_none_or(|v| v.mode != crate::mask_view_cmds::MaskViewMode::Gray));
    }
    Ok(match crate::channel_cmds::target_of(p) {
        crate::channel_cmds::Target::Mask => crate::mask_view_cmds::current(st).is_some_and(|v| !v.filter),
        crate::channel_cmds::Target::Alpha(i) => st.channel_view.alpha_shown(i),
        crate::channel_cmds::Target::QuickMask => !st.channel_view.quick_mask_hidden,
        _ => true,
    })
}

/// Scalar source at the document's native sample depth; None means an ordinary layer transform.
pub fn plane(doc: &Document, layer: Option<LayerId>, p: &Value, background: [f32; 4]) -> Result<Option<Surface>> {
    let Some(k) = color_index(p)? else {
        return crate::transform_cmds::lone_target(doc, layer, p).map(|s| s.cloned());
    };
    let id = layer.ok_or_else(|| bad("no active layer"))?;
    let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Raster(source) = &l.content else {
        return Err(bad("individual colour transforms require a raster layer"));
    };
    let fmt = source.format();
    if k >= fmt.mode.color_channels() {
        return Err(bad("colour channel index is out of range"));
    }
    let space = photocraft_compose::cmyk_space(doc);
    let native = photocraft_color::convert::with_cmyk_space(space.as_ref(), || from_rgba(&fmt, background));
    let fill = native.get(k).copied().ok_or_else(|| bad("missing background component"))?;
    let mut out = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, fmt.sample, false), &[fill]);
    let area = doc.bounds();
    validate_area(area)?;
    for tile in area.tiles() {
        let r = tile.rect().intersect(&area);
        let values = crate::allocation::read_region(source, r, "extracting a transform channel")?;
        let mut gray = crate::allocation::capacity(values.len() / fmt.channels(), "extracting a transform channel")?;
        for pixel in values.chunks_exact(fmt.channels()) {
            gray.push(pixel.get(k).copied().ok_or_else(|| bad("missing colour sample"))?);
        }
        out.try_write_region(r, &gray)?;
    }
    Ok(Some(out))
}

/// Resolve and validate an interactive scalar frame without allocating its full-resolution source.
pub fn source_bounds(doc: &Document, layer: Option<LayerId>, p: &Value) -> Result<Option<Rect>> {
    check_locks(doc, layer, p)?;
    if let Some(selection) = &doc.selection {
        validate_area(selection.content_bounds())?;
    }
    if let Some(k) = color_index(p)? {
        let id = layer.ok_or_else(|| bad("no active layer"))?;
        let surface = doc
            .layer(id)
            .and_then(|l| if let LayerContent::Raster(s) = &l.content { Some(s) } else { None })
            .ok_or_else(|| bad("individual colour transforms require a raster layer"))?;
        if k >= surface.format().mode.color_channels() {
            return Err(bad("colour channel index is out of range"));
        }
        validate_area(doc.bounds())?;
        return bounds(doc, surface, p).map(Some);
    }
    crate::transform_cmds::lone_target(doc, layer, p)?
        .map(|surface| {
            validate_area(surface.content_bounds())?;
            bounds(doc, surface, p)
        })
        .transpose()
}

/// Reject enormous scalar buffers before a resampler allocates them.
pub(crate) fn validate_area(r: Rect) -> Result<()> {
    if u64::from(r.width()).saturating_mul(u64::from(r.height())) > 100_000_000 {
        return Err(bad("channel transform exceeds the 100 megapixel working area limit"));
    }
    Ok(())
}

pub fn bounds(doc: &Document, plane: &Surface, p: &Value) -> Result<Rect> {
    Ok(if color_index(p)?.is_some() {
        doc.selection.as_ref().map_or(doc.bounds(), |s| s.content_bounds().intersect(&doc.bounds()))
    } else if let Some(selection) = &doc.selection {
        // Default-valued selected mask/channel samples move too, so the frame must contain them.
        selection.content_bounds()
    } else {
        crate::transform_cmds::target_bounds(doc, plane)
    })
}

/// Check geometric/pixel locks without interpreting a Background's structural lock as a user lock.
pub fn check_locks(doc: &Document, layer: Option<LayerId>, p: &Value) -> Result<()> {
    if color_index(p)?.is_none() && !crate::commands::is_mask_target(p) {
        return Ok(());
    }
    let id = layer.ok_or_else(|| bad("no active layer"))?;
    let mut copy = doc.clone();
    crate::extra_cmds::background_to_layer(&mut copy, id);
    let locks = copy.effective_locks(id);
    if locks.all || locks.position || (color_index(p)?.is_some() && locks.pixels) {
        return Err(bad("the target layer is locked"));
    }
    Ok(())
}

/// Replace only the native encoded bytes of the edited component. Other components stay bit-exact.
pub fn write_plane(doc: &mut Document, layer: Option<LayerId>, p: &Value, plane: Surface) -> Result<()> {
    check_locks(doc, layer, p)?;
    let Some(k) = color_index(p)? else {
        let (dst, _) = crate::channel_cmds::target_surface(doc, layer, p)?;
        *dst = plane;
        return Ok(());
    };
    let area = doc.bounds();
    let id = layer.ok_or_else(|| bad("no active layer"))?;
    let dst = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or(EngineError::NoLayer(id))?;
    let fmt = dst.format();
    if k >= fmt.mode.color_channels() || plane.format().sample != fmt.sample || plane.format().mode != ColorMode::Grayscale || plane.channels() != 1 {
        return Err(bad("incompatible transform channel"));
    }
    let bytes = fmt.sample.bytes();
    let offset = k.checked_mul(bytes).ok_or_else(|| bad("colour offset overflow"))?;
    for tile in area.tiles() {
        let r = tile.rect().intersect(&area);
        let mut pixels = dst.try_to_interleaved(r)?;
        let values = plane.try_to_interleaved(r)?;
        for (pixel, value) in pixels.chunks_exact_mut(fmt.bytes_per_pixel()).zip(values.chunks_exact(bytes)) {
            let slot = pixel.get_mut(offset..offset + bytes).ok_or_else(|| bad("missing colour sample"))?;
            slot.copy_from_slice(value);
        }
        // The region is tile-bounded and bytes were fallibly allocated at its exact encoded size.
        dst.try_tile_mut(tile)?;
        dst.write_interleaved(r, &pixels);
    }
    dst.prune();
    Ok(())
}

#[cfg(test)]
mod tests;
