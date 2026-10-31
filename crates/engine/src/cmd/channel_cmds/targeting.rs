use super::manage::fix_view_now;
use super::planes::{color_count, materialize};
use super::refs::{ChanRef, bad, parse_ref};
use super::view::ChannelTarget;
use super::{ColorMode, Document, EngineError, Layer, LayerId, Rect, Result, Session, Surface, TileCoord, Value, from_rgba, json, list};

// ---------------------------------------------------------------------------------------------
// Targets for pixel commands

/// Where a pixel command writes, from its `"target"` param.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Pixels,
    Mask,
    /// A smart object's filter mask (`"target":"filterMask"`): where it is black the smart
    /// filters don't show (FILE-215-3).
    FilterMask,
    Alpha(usize),
    QuickMask,
}

pub(crate) fn target_of(p: &Value) -> Target {
    match p.get("target") {
        Some(Value::String(s)) => match s.as_str() {
            "mask" => Target::Mask,
            "filterMask" => Target::FilterMask,
            "quickMask" => Target::QuickMask,
            _ => Target::Pixels,
        },
        Some(Value::Object(o)) => o.get("channel").and_then(Value::as_u64).map_or(Target::Pixels, |i| Target::Alpha(i as usize)),
        _ => Target::Pixels,
    }
}

/// The target is an alpha channel or the Quick Mask (no layer involved).
pub(crate) fn is_channel_target(p: &Value) -> bool {
    matches!(target_of(p), Target::Alpha(_) | Target::QuickMask)
}

/// The target is a spot channel: it stores ink but takes tone, as it reads.
pub(crate) fn is_spot_target(doc: &Document, p: &Value) -> bool {
    matches!(target_of(p), Target::Alpha(i) if doc.channels.get(i).is_some_and(|c| c.spot.is_some()))
}

/// Inverts `r` of a channel surface (ink to tone and back).
pub(crate) fn invert_region(s: &mut Surface, r: Rect) -> Result<()> {
    let mut v = crate::allocation::read_region(s, r, "inverting a channel")?;
    v.iter_mut().for_each(|x| *x = 1.0 - *x);
    s.try_write_region(r, &v)?;
    Ok(())
}

/// The surface a pixel command writes to and whether the layer's transparency is locked.
pub(crate) fn target_surface<'a>(doc: &'a mut Document, layer: Option<LayerId>, p: &Value) -> Result<(&'a mut Surface, bool)> {
    match target_of(p) {
        Target::Alpha(i) => {
            let n = doc.channels.len();
            doc.channels.get_mut(i).map(|c| (&mut c.surface, false)).ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {n})")))
        }
        Target::QuickMask => doc.quick_mask.as_mut().map(|c| (&mut c.surface, false)).ok_or_else(|| EngineError::Other("not in Quick Mask mode".into())),
        t => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let lock = doc.effective_locks(id).transparency && t == Target::Pixels;
            Ok((crate::commands::paint_surface(doc, id, p)?, lock))
        }
    }
}

pub(crate) fn read_target_surface<'a>(doc: &'a Document, layer: Option<LayerId>, p: &Value) -> Result<&'a Surface> {
    match target_of(p) {
        Target::Alpha(i) => {
            let n = doc.channels.len();
            doc.channels.get(i).map(|c| &c.surface).ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {n})")))
        }
        Target::QuickMask => doc.quick_mask.as_ref().map(|c| &c.surface).ok_or_else(|| EngineError::Other("not in Quick Mask mode".into())),
        Target::Pixels => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            doc.layer(id).and_then(Layer::surface).ok_or(EngineError::NoLayer(id))
        }
        Target::Mask => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            doc.layer(id).and_then(|l| l.mask.as_ref()).map(|m| m.paint_surface()).ok_or(EngineError::NoLayer(id))
        }
        Target::FilterMask => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            doc.layer(id)
                .and_then(|l| crate::mask_view_cmds::viewed_mask(l, true))
                .map(|m| &m.surface)
                .ok_or_else(|| EngineError::Other("the layer has no filter mask".into()))
        }
    }
}

/// The grayscale surface filters and adjustments edit instead of the layer's pixels (made explicit
/// over the canvas), if the params target one: an alpha channel, the Quick Mask, or the mask of
/// `layer`.
pub(crate) fn channel_surface_for_filter<'a>(doc: &'a mut Document, layer: Option<LayerId>, p: &Value) -> Result<Option<&'a mut Surface>> {
    if target_of(p) == Target::Pixels {
        return Ok(None);
    }
    let b = doc.bounds();
    let (surf, _) = target_surface(doc, layer, p)?;
    materialize(surf, b);
    Ok(Some(surf))
}

/// Commands that edit a targeted layer mask in place of the layer: the Image › Adjustments table,
/// the filters and the fills (Edit › Fill and its keys). A mask target enables them on any layer
/// with a mask, so ⌘I inverts an adjustment layer's mask (#780) and Fill fills it (#2456) as in
/// Photoshop.
fn edits_mask(id: &str) -> bool {
    id.strip_prefix("image.adjustments.").is_some_and(|kind| crate::adjust_params::default_for(kind, ColorMode::Rgb).is_ok())
        || crate::filters::params_for(id, &Value::Null).is_some()
        || id == "edit.fill"
        // Copying a targeted mask (#2928).
        || id == "edit.copy"
        || crate::fill_key_cmds::IDS.contains(&id)
}

/// The precondition of `id` when `p` targets the active layer's mask (`"target":"mask"`): the
/// layer needs a mask, and non-brush edits cannot write into an editable Paint stack.
pub(crate) fn mask_target_enabled(s: &Session, id: &str, p: &Value) -> Option<std::result::Result<(), String>> {
    match target_of(p) {
        // The commands that paint, fill, filter or adjust the filter mask need a smart object with
        // smart filters, not a pixel layer. Paste keeps its own check (something to paste).
        Target::FilterMask if follows_filter_mask(id) && !PASTE_IDS.contains(&id) => {
            Some(crate::active_layer_of(s).and_then(crate::smart_cmds::filter_mask_paintable))
        }
        Target::Mask if edits_mask(id) || follows_target(id) => {
            Some(crate::active_layer_of(s).and_then(|l| {
                let mask = l.mask.as_ref().ok_or_else(|| "the active layer has no layer mask".to_string())?;
                // Brush, Pencil and tangent-wrap write to the selected Paint layer's RGBA
                // surface. Copy reads the flattened mask and leaves the editable stack intact.
                // Other routed commands write a grayscale surface and would discard its effect
                // semantics; brush tools must also reject a selected Fill/Levels/Blur effect.
                let paint_stroke = matches!(id, "paint.stroke" | "paint.pencil" | "paint.uvWrap")
                    && mask
                        .paint
                        .as_ref()
                        .and_then(|stack| stack.layers.get(stack.active))
                        .is_some_and(|layer| matches!(&layer.kind, photocraft_doc::mask_paint::EffectKind::Paint));
                if mask.paint.is_some() && id != "edit.copy" && !paint_stroke {
                    return Err("Select a Paint sublayer before using Brush, Pencil or tangent-wrap on an editable mask stack".into());
                }
                Ok(())
            }))
        }
        _ => None,
    }
}

/// Geometry commands that transform the flattened mask surface but do not yet carry editable
/// mask Paint data through the same transform. Refuse before running them against a cloned doc.
pub(crate) fn mask_stack_geometry_enabled(s: &Session, id: &str, p: &Value) -> Option<std::result::Result<(), String>> {
    let changes_document_geometry = matches!(
        id,
        "image.imageSize"
            | "image.canvasSize"
            | "image.crop"
            | "image.trim"
            | "image.rotation.arbitrary"
            | "image.imageRotation.flipCanvasHorizontal"
            | "image.imageRotation.flipCanvasVertical"
            | "image.imageRotation.180"
            | "image.imageRotation.90cw"
            | "image.imageRotation.90ccw"
    );
    let changes_layer_geometry = id == "layer.translate";
    if !changes_document_geometry && !changes_layer_geometry {
        return None;
    }
    let has_editable_mask_stack = s.active().is_some_and(|state| {
        let walk = state.doc.walk();
        if changes_document_geometry {
            return walk.iter().any(|(_, _, layer)| layer.mask.as_ref().is_some_and(|mask| mask.paint.is_some()));
        }

        // `layer.translate` may move an explicit layer, a multi-selection or its linked layers.
        // Include descendants of moved groups. A stack is affected only if its mask moves with
        // the pixels, or if the command explicitly moves that layer's unlinked mask by itself.
        let roots = crate::commands::explicit_layer(s, p).ok().flatten().map(|id| vec![id]).unwrap_or_else(|| crate::layer_multi_cmds::selected(s));
        if roots.is_empty() {
            return false;
        }
        let moved = crate::layer_multi_cmds::move_targets(&state.doc, &roots);
        let moved_paths: Vec<_> = walk.iter().filter(|(_, _, layer)| moved.contains(&layer.id)).map(|(path, _, _)| path.to_vec()).collect();
        let moves_mask = target_of(p) == Target::Mask;
        walk.iter().any(|(path, _, layer)| {
            let mask_moves_with_layer = moved_paths.iter().any(|moved_path| path.starts_with(moved_path.as_slice()));
            layer
                .mask
                .as_ref()
                .is_some_and(|mask| mask.paint.is_some() && ((mask_moves_with_layer && mask.linked) || (moves_mask && roots.contains(&layer.id))))
        })
    });
    has_editable_mask_stack
        .then(|| Err("This geometry change is disabled while the affected editable mask Paint stack exists; preserve or remove the stack first".into()))
}

/// Whether command `id` edits the targeted channel or mask when its params name no `"target"`
/// (filters, adjustments, paint and fill commands); the shell adds its mask target to these.
pub fn follows_target(id: &str) -> bool {
    // Moving the selected pixels (⌘-drag, Move tool) moves the targeted mask's, not the layer's.
    // Moving a layer with its unlinked mask targeted moves the mask alone (`layer.translate`).
    routed(id) || id == "select.float" || id == "layer.translate"
}

/// Whether command `id` edits a smart object's targeted filter mask (`"target":"filterMask"`):
/// every routed command but those that act on the smart object itself while its filter mask is
/// targeted (Edit › Transform, Apply Image, Convert for Smart Filters, Last Filter, Liquify, the
/// lens filters and Vanishing Point, which add a smart filter, the render filters that make a new
/// layer, and Desaturate), so neither the shell nor the engine sends those the filter mask.
pub fn follows_filter_mask(id: &str) -> bool {
    routed(id)
        && !id.starts_with("edit.transform.")
        && !matches!(
            id,
            "image.applyImage"
                | "image.adjustments.desaturate"
                | "filter.vanishingPoint"
                | "filter.render.flame"
                | "filter.render.tree"
                | "filter.render.pictureFrame"
                | "filter.convertForSmartFilters"
                | "filter.lastFilter"
                | crate::distort_cmds::LIQUIFY
                | crate::lens_cmds::LENS
                | crate::lens_cmds::WIDE
                | crate::lens_cmds::RAW
        )
}

/// The paint and retouch tools that follow the targeted channel or mask.
const PAINT_IDS: &[&str] = &[
    "paint.uvWrap",
    "paint.stroke",
    "paint.pencil",
    "paint.bucket",
    "paint.gradient",
    "paint.mixerBrush",
    "paint.cloneStamp",
    "paint.patternStamp",
    "paint.healingBrush",
    "paint.spotHealing",
    "paint.dodge",
    "paint.burn",
    "paint.sponge",
    "paint.blur",
    "paint.sharpen",
    "paint.smudge",
    "paint.historyBrush",
    "paint.artHistoryBrush",
    "paint.redEye",
    "paint.remove",
];

/// Pasting into a targeted mask or channel (#1035).
const PASTE_IDS: &[&str] = &["edit.paste", "edit.pasteSpecial.pasteInPlace", "edit.pasteSpecial.pasteInto", "edit.pasteSpecial.pasteOutside"];

/// Commands whose target follows the Channels panel.
pub(crate) fn routed(id: &str) -> bool {
    id.starts_with("filter.")
        || id == "plugin.run"
        || id.starts_with("image.adjustments.")
        || matches!(id, "paint.uvWrap" | "edit.fill" | "image.applyImage")
        || crate::fill_key_cmds::IDS.contains(&id)
        || PAINT_IDS.contains(&id)
        // Pasting into a targeted mask or channel (#1035); Copy takes a targeted mask (#2928).
        || id == "edit.copy"
        || PASTE_IDS.contains(&id)
        // Edit › Transform › Rotate/Flip turn a targeted mask or channel (#2835).
        || matches!(
            id,
            "edit.transform.rotate180"
                | "edit.transform.rotate90Cw"
                | "edit.transform.rotate90Ccw"
                | "edit.transform.flipHorizontal"
                | "edit.transform.flipVertical"
        )
}

/// Fill in `"target"` from the targeted channel (or Quick Mask mode) when the caller gave none.
pub(crate) fn inject_target(s: &Session, id: &str, params: Value) -> Value {
    if !routed(id) || params.get("target").is_some() {
        return params;
    }
    let Some(st) = s.active() else { return params };
    let t = match st.channel_view.target {
        ChannelTarget::Alpha(i) if i < st.doc.channels.len() => json!({ "channel": i }),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => json!("quickMask"),
        ChannelTarget::LayerMask => json!("mask"),
        // Viewing the active layer's mask (⌥-click): pixel commands edit the mask.
        // A shown filter mask takes only the commands that edit it; the rest act on the smart object.
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some_and(|v| v.filter) => {
            if !follows_filter_mask(id) {
                return params;
            }
            json!("filterMask")
        }
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some() => json!("mask"),
        _ => return params,
    };
    match params {
        Value::Object(mut m) => {
            m.insert("target".into(), t);
            Value::Object(m)
        }
        _ => json!({ "target": t }),
    }
}

/// The smart object whose filter mask command `id` with `params` edits (`"target":"filterMask"`):
/// its `"layer"` param, else the active layer. None for a command that doesn't edit the filter
/// mask, so moving the smart object doesn't re-render its filters.
pub(crate) fn filter_mask_layer(s: &Session, id: &str, params: &Value) -> Option<LayerId> {
    if target_of(params) != Target::FilterMask || !follows_filter_mask(id) {
        return None;
    }
    params.get("layer").and_then(Value::as_u64).map(LayerId).or_else(|| s.active()?.active_layer)
}

/// The colour channel pixel commands are limited to, if a single colour channel is targeted.
pub(crate) fn color_restriction(s: &Session, id: &str, params: &Value) -> Option<usize> {
    if !routed(id) || is_channel_target(params) || crate::commands::is_mask_target(params) {
        return None;
    }
    match s.active()?.channel_view.target {
        ChannelTarget::Color(k) => Some(k),
        _ => None,
    }
}

/// After a pixel edit with colour channel `k` targeted, put the other channels (and the layer's
/// transparency) of layer `id` back as they were. Only changed tiles are touched.
pub(crate) fn restrict_to_color(before: &Document, after: &mut Document, id: LayerId, k: usize) {
    let Some(old) = before.layer(id).and_then(Layer::surface) else { return };
    let Some(new) = after.layer_mut(id).and_then(Layer::surface_mut) else { return };
    if old.format() != new.format() {
        return;
    }
    let mut coords: Vec<TileCoord> = new.tiles().filter(|(c, t)| old.tile(**c).is_none_or(|o| !std::sync::Arc::ptr_eq(o, t))).map(|(c, _)| *c).collect();
    coords.extend(old.tiles().filter(|(c, _)| new.tile(**c).is_none()).map(|(c, _)| *c));
    let n = new.channels();
    for c in coords {
        let r = c.rect();
        let o = old.read_region(r);
        let mut v = new.read_region(r);
        for (pn, po) in v.chunks_exact_mut(n).zip(o.chunks_exact(n)) {
            for ch in 0..n {
                if ch != k {
                    pn[ch] = po[ch];
                }
            }
        }
        new.write_region(r, &v);
    }
    new.prune();
}

/// The colour channel Clear is limited to: the Channels panel targets a single one.
pub(crate) fn targeted_color(s: &Session) -> Option<usize> {
    match s.active()?.channel_view.target {
        ChannelTarget::Color(k) => Some(k),
        _ => None,
    }
}

/// Clear (Delete) with colour channel `k` of layer `id` targeted (#2720): fill the selected area
/// (or the canvas) of that channel only with the background colour's value for it, as Photoshop
/// does. A channel has no transparency of its own, so the other channels and the layer's alpha
/// stay; partly selected pixels blend.
pub(crate) fn clear_color_channel(doc: &mut Document, id: LayerId, k: usize, background: [f32; 4]) -> Result<()> {
    let canvas = doc.bounds();
    let sel = doc.selection.clone();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
    let fmt = surf.format();
    let n = fmt.channels();
    // The background colour in the layer's colour model, so `k` picks the right component.
    let Some(v) = from_rgba(&fmt, [background[0], background[1], background[2], 1.0]).get(k).copied().filter(|_| k < n) else {
        return Err(EngineError::Other(format!("no colour channel {k} in this layer")));
    };
    if area.is_empty() {
        return Ok(());
    }
    let cover =
        sel.as_ref().map(|m| crate::allocation::read_region(m, area, "clearing a channel selection").map(|v| (v, m.format().channels().max(1)))).transpose()?;
    let mut px = crate::allocation::read_region(surf, area, "clearing a channel")?;
    for (i, p) in px.chunks_exact_mut(n).enumerate() {
        let w = cover.as_ref().map_or(1.0, |(m, mk)| m.get(i * mk).copied().unwrap_or(0.0));
        if let Some(c) = p.get_mut(k) {
            *c += (v - *c) * w.clamp(0.0, 1.0);
        }
    }
    surf.try_write_region(area, &px)?;
    surf.prune();
    Ok(())
}

/// The alpha channel or Quick Mask the Channels panel targets, as `"target"` params: Clear
/// (Delete) fills it instead of the layer (#2734).
pub(crate) fn targeted_channel(s: &Session) -> Option<Value> {
    let st = s.active()?;
    let t = match st.channel_view.target {
        ChannelTarget::Alpha(i) if i < st.doc.channels.len() => json!({ "channel": i }),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => json!("quickMask"),
        _ => return None,
    };
    Some(json!({ "target": t }))
}

/// Clear (Delete) with an alpha channel or the Quick Mask targeted (`target` from
/// [`targeted_channel`]): fill the selected area (or the canvas) of it with the background colour,
/// as gray, as Photoshop does. Partly selected pixels blend.
pub(crate) fn clear_channel(doc: &mut Document, target: &Value, background: [f32; 4]) -> Result<()> {
    let canvas = doc.bounds();
    let sel = doc.selection.clone();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    if area.is_empty() {
        return Ok(());
    }
    let (surf, _) = target_surface(doc, None, target)?;
    crate::fill_cmds::fill_color(surf, area, [background[0], background[1], background[2], 1.0], sel.as_ref())
}

/// The active document targets an alpha channel or is in Quick Mask mode, so pixel commands
/// work without a pixel layer.
pub(crate) fn edits_channel(s: &Session) -> bool {
    s.active().is_some_and(|st| {
        st.doc.quick_mask.is_some() && st.channel_view.target == ChannelTarget::Composite
            || matches!(st.channel_view.target, ChannelTarget::Alpha(i) if i < st.doc.channels.len())
    })
}

/// Make `t` the edit target and show it the way Photoshop does when its row is clicked.
pub(super) fn target(s: &mut Session, t: ChannelTarget) {
    if let Some(st) = s.active_mut() {
        let colors = color_count(&st.doc);
        let n = st.doc.channels.len();
        let v = &mut st.channel_view;
        v.target = t;
        match t {
            // Like clicking RGB in Photoshop: the alpha overlays go away with their eyes.
            ChannelTarget::Composite => {
                v.color_hidden.clear();
                v.alpha_visible.clear();
            }
            ChannelTarget::LayerMask | ChannelTarget::VectorMask => v.color_hidden.clear(),
            ChannelTarget::Color(k) => {
                v.color_hidden = (0..colors).map(|c| c != k).collect();
                v.alpha_visible.clear();
            }
            ChannelTarget::Alpha(i) => {
                v.color_hidden = vec![true; colors];
                v.alpha_visible = (0..n).map(|c| c == i).collect();
            }
        }
        fix_view_now(st);
    }
    s.tools.target_mask(t == ChannelTarget::LayerMask);
}

fn parse_target(s: &Session, p: &Value, cmd: &str) -> Result<ChannelTarget> {
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    match p.get("channel").map_or(Some(ChanRef::Composite), |v| parse_ref(v, doc)) {
        Some(ChanRef::Composite) => Ok(ChannelTarget::Composite),
        Some(ChanRef::Color(k)) if k < color_count(doc) && color_count(doc) > 1 => Ok(ChannelTarget::Color(k)),
        Some(ChanRef::Alpha(i)) if i < doc.channels.len() => Ok(ChannelTarget::Alpha(i)),
        Some(ChanRef::LayerMask) => {
            let active = crate::active_layer_of(s).map_err(|e| bad(cmd, e))?;
            if active.mask.is_none() {
                return Err(bad(cmd, "the active layer has no layer mask"));
            }
            Ok(ChannelTarget::LayerMask)
        }
        Some(ChanRef::VectorMask) => {
            let active = crate::active_layer_of(s).map_err(|e| bad(cmd, e))?;
            if active.vector_mask.is_none() {
                return Err(bad(cmd, "the active layer has no vector mask"));
            }
            Ok(ChannelTarget::VectorMask)
        }
        _ => Err(bad(cmd, "`channel` must be \"composite\", \"mask\", \"vectorMask\", a colour channel name or an alpha channel")),
    }
}

pub(super) fn target_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let t = parse_target(s, p, "channel.target")?;
    target(s, t);
    Ok(list(s))
}

/// The channel shortcuts' modifier as the platform writes it, for error messages (`⌘4`, `Ctrl+4`).
pub(super) const SLOT_KEY: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

/// ⌘2 composite, ⌘3… colour channels then alpha channels (Photoshop CC defaults).
pub(super) fn target_slot(s: &mut Session, slot: usize) -> Result<Value> {
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let colors = color_count(doc);
    let shown_colors = if colors > 1 { colors } else { 0 };
    let t = match slot {
        2 => ChannelTarget::Composite,
        n if n - 3 < shown_colors => ChannelTarget::Color(n - 3),
        n if n - 3 - shown_colors < doc.channels.len() => ChannelTarget::Alpha(n - 3 - shown_colors),
        _ => return Err(EngineError::Other(format!("no channel for {SLOT_KEY}{slot}"))),
    };
    target(s, t);
    Ok(list(s))
}
