//! Everyday Photoshop commands: clipboard (Cut/Copy/Copy Merged/Paste/Paste in Place), Layer via
//! Copy/Cut, Merge Visible, Auto Tone/Contrast/Color, Toggle Last State and Transform Again.

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::adjust::LevelsChannel;
use photocraft_doc::{Adjustment, DocId, Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The paste commands' `"target"` param (for `concat!` into their params docs).
macro_rules! paste_target {
    () => {
        r#""target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target (not pixels: the luminosity pasted into that mask or channel)"#
    };
}
pub(crate) use paste_target;

/// Pixels on the session clipboard, with where they came from (for Paste in Place).
#[derive(Clone, Debug)]
pub struct Clip {
    pub surface: Surface,
    pub bounds: Rect,
    /// The copied layer itself, when Copy took a whole layer that Paste should recreate rather
    /// than rasterize (see [`whole_layer`]): a scratch document in the source's colour holding it.
    /// `surface` still holds its pixels, for Paste Into, masks, channels and other apps.
    pub layers: Option<Box<Document>>,
    /// The source document's resolution (ppi): File › New from the clipboard takes it, as
    /// Photoshop's Clipboard preset does. None for an image from another app.
    pub resolution: Option<f32>,
    /// The document the pixels were copied from: Paste keeps them in place only there, and
    /// centres them in any other (#8945). None for an image from another app.
    pub source: Option<DocId>,
}

/// Photoshop's layer-based copy (CC 2018 on): with no selection, Copy takes the active layer
/// whole when it is a smart object (#2428), a type or shape layer, or a group (its whole subtree)
/// (#2670), so Paste recreates it instead of its pixels. Fill and adjustment layers have no
/// pixels of their own, so they are taken whole too rather than refused (#2344). A pixel layer
/// with a raster or vector mask is also taken whole to keep that mask editable (#2184); an
/// unmasked pixel layer still copies its pixels. Several selected layers are [`multi_copy_ids`].
fn takes_whole(d: &crate::DocState, id: LayerId) -> bool {
    d.doc.selection.is_none()
        && d.selected_layers().len() <= 1
        && crate::layer_copy_cmds::layered(d.doc.mode)
        && d.doc.layer(id).is_some_and(|l| !matches!(l.content, LayerContent::Raster(_)) || l.mask.is_some() || l.vector_mask.is_some())
}

/// A scratch document in the source's colour, to hold copied layers.
fn scratch_doc(doc: &Document) -> Document {
    let mut copies = Document::new("", doc.size, doc.mode, doc.depth);
    copies.icc_profile = doc.icc_profile.clone();
    copies.resolution_dpi = doc.resolution_dpi;
    copies.patterns = doc.patterns.clone();
    copies
}

/// Scope linked records to the copied layer tree: a PSD block can also hold files belonging
/// to unselected or hidden layers, which must not follow this copy into a native save.
fn copy_linked_sources(source: &Document, copies: &mut Document) -> Result<()> {
    fn collect<'a>(layers: &'a [Layer], depth: usize, paths: &mut std::collections::HashSet<&'a str>) -> Result<()> {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(EngineError::Other("can't copy linked smart objects from layers nested too deeply".into()));
        }
        for layer in layers {
            if let LayerContent::Smart(sm) = &layer.content
                && let photocraft_doc::SmartSource::Linked { path } = &sm.source
            {
                paths.try_reserve(1).map_err(|_| EngineError::Other("not enough memory to copy smart-object source IDs".into()))?;
                paths.insert(path);
            }
            if let Some(children) = layer.children().filter(|children| !children.is_empty()) {
                collect(children, depth + 1, paths)?;
            }
        }
        Ok(())
    }
    let mut paths = std::collections::HashSet::new();
    collect(&copies.layers, 0, &mut paths)?;
    if paths.is_empty() {
        return Ok(());
    }
    for (signature, key, data) in &source.metadata.psd_global_blocks {
        if !matches!(key, b"lnk2" | b"lnk3" | b"lnkD" | b"lnkE") {
            continue;
        }
        let selected = photocraft_io::linked::select_items(data, &|uuid, _| paths.contains(uuid))
            .map_err(|e| EngineError::Other(format!("Could not copy smart-object contents: {e}; the clipboard was not changed")))?;
        let selected = selected.map(std::sync::Arc::new).unwrap_or_else(|| data.clone());
        if !selected.is_empty() {
            copies.metadata.psd_global_blocks.try_reserve(1).map_err(|_| EngineError::Other("not enough memory to copy smart-object source records".into()))?;
            copies.metadata.psd_global_blocks.push((*signature, *key, selected));
        }
    }
    Ok(())
}

/// The layer Copy takes whole ([`takes_whole`]): a scratch document in the source's colour
/// holding it.
fn whole_layer(d: &crate::DocState, id: LayerId) -> Option<Box<Document>> {
    let l = d.doc.layer(id).filter(|_| takes_whole(d, id))?;
    let mut copies = scratch_doc(&d.doc);
    copies.layers.push(l.clone());
    Some(Box::new(copies))
}

/// The layers Copy takes when several are selected and nothing is selected in the pixels
/// (Photoshop, #2478): the selected layers that no selected group already carries, bottom-to-top.
/// Empty otherwise, and for a single layer ([`takes_whole`] decides that case).
fn multi_copy_ids(d: &crate::DocState) -> Vec<LayerId> {
    if d.doc.selection.is_some() || !crate::layer_copy_cmds::layered(d.doc.mode) {
        return Vec::new();
    }
    let selected = d.selected_layers();
    if selected.len() < 2 {
        return Vec::new();
    }
    crate::layer_multi_cmds::top_level(&d.doc, &selected)
}

/// Every selected layer, whole ([`multi_copy_ids`]), in a scratch document: Paste recreates them
/// all, each with its mask, styles and content. The Background becomes an ordinary layer, as in
/// Photoshop's Duplicate Layer.
fn whole_layers(d: &crate::DocState, ids: &[LayerId]) -> Option<Box<Document>> {
    let mut copies = scratch_doc(&d.doc);
    for id in ids {
        let mut l = d.doc.layer(*id)?.clone();
        if d.doc.is_background(*id) {
            l.name = "Layer 0".into();
            l.locks = Default::default();
        }
        copies.layers.push(l);
    }
    Some(Box::new(copies))
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Paste and New from Clipboard need only a clipboard: with no document open, they make one.
fn has_clip_only(s: &Session) -> std::result::Result<(), String> {
    s.clipboard.as_ref().map(|_| ()).ok_or_else(|| "the clipboard is empty".into())
}

/// Validate the copied layer tree before its recursive Clone implementation runs.
fn clipboard_snapshot(s: &Session) -> Result<Clip> {
    let clip = s.clipboard.as_ref().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    if let Some(layers) = clip.layers.as_deref() {
        crate::cmd::layer_multi_cmds::check_group_depth(layers, "Paste")?;
    }
    Ok(clip.clone())
}

fn active_layer(s: &Session) -> std::result::Result<&Layer, String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.and_then(|id| d.doc.layer(id)).ok_or_else(|| "no active layer".into())
}

/// Cut and Layer via Cut edit the pixels, so they need a pixel layer.
fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    match active_layer(s)?.content {
        LayerContent::Raster(_) => Ok(()),
        LayerContent::Smart(_) => Err("the smart object is not directly editable".into()),
        _ => Err("the active layer has no pixels".into()),
    }
}

/// Copy only reads: any layer that shows pixels will do, so a smart object, type or shape layer
/// copies what it shows (Photoshop).
fn has_layer_pixels(s: &Session) -> std::result::Result<(), String> {
    active_layer(s)?.surface().map(|_| ()).ok_or_else(|| "the active layer has no pixels".into())
}

/// Copy: a targeted alpha channel (#2307), a layer with pixels, or one taken whole (a group with
/// no selection, #2670).
fn can_copy(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_clip::copies_alpha_channel(s) || s.active().is_some_and(|d| !multi_copy_ids(d).is_empty()) {
        return Ok(());
    }
    has_layer_pixels(s).or_else(|e| s.active().and_then(|d| d.active_layer.filter(|id| takes_whole(d, *id))).map(|_| ()).ok_or(e))
}

fn has_clip(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    s.clipboard.as_ref().map(|_| ()).ok_or_else(|| "the clipboard is empty".into())
}

fn active_id(s: &Session) -> Result<LayerId> {
    s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into()))
}

/// The pixels of `src` inside the selection (or all of them), as a straight-alpha surface.
///
/// Built tile by tile (#4516). A source tile that the copy takes whole (inside the canvas, fully
/// selected or no selection, already in the clip's format) is shared copy-on-write, so a plain
/// Copy or Select All, Copy costs a pointer per tile; only the other tiles are read and written,
/// one at a time, instead of the whole area as one `f32` buffer.
fn lift(src: &Surface, sel: Option<&Surface>, canvas: Rect) -> Result<Clip> {
    let fmt = src.format();
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let area = match sel {
        Some(m) => m.content_bounds().intersect(&canvas),
        None => src.content_bounds().intersect(&canvas),
    };
    let mut out = Surface::new(with_alpha);
    if area.is_empty() {
        return Ok(Clip { surface: out, bounds: Rect::EMPTY, layers: None, resolution: None, source: None });
    }
    // Only the selected area is read: converting the whole layer (a 24 MP Background gaining
    // alpha) cost about a second per Layer via Copy (#668).
    let mut shared = Vec::new();
    for tc in area.tiles() {
        let tr = tc.rect().intersect(&area);
        if fmt.alpha
            && tr == tc.rect()
            && let Some(tile) = src.tile(tc)
            && sel.is_none_or(|m| crate::fill_cmds::Mask::of(m, tc).full(tr))
        {
            // Reading and writing it back would give the same bytes.
            shared.push((tc, tile.clone()));
            continue;
        }
        let px = lift_region(src, sel, tr, with_alpha.channels())?;
        out.try_write_region(tr, &px).map_err(|e| EngineError::Other(format!("not enough memory while copying pixels ({e}); the document was not changed")))?;
    }
    out.put_tiles(shared);
    out.prune();
    let bounds = out.content_bounds();
    Ok(Clip { surface: out, bounds, layers: None, resolution: None, source: None })
}

/// [`lift`] over one rectangle: `src`'s pixels with alpha (`n` channels), the selection
/// multiplied into it.
fn lift_region(src: &Surface, sel: Option<&Surface>, r: Rect, n: usize) -> Result<Vec<f32>> {
    let fmt = src.format();
    let mut px = if fmt.alpha {
        crate::allocation::read_region(src, r, "copying pixels")?
    } else {
        let opaque = crate::allocation::read_region(src, r, "copying pixels")?;
        let k = fmt.channels();
        let len = opaque
            .len()
            .checked_div(k.max(1))
            .and_then(|pixels| pixels.checked_mul(n))
            .ok_or_else(|| EngineError::Other("not enough memory for copying pixels (requested size overflowed); the document was not changed".into()))?;
        let mut v = crate::allocation::filled(len, 0.0, "copying pixels")?;
        let mut at = 0;
        for p in opaque.chunks_exact(k.max(1)) {
            let Some(dst) = v.get_mut(at..at + k.max(1)) else { break };
            dst.copy_from_slice(p);
            at += k.max(1);
            if let Some(alpha) = v.get_mut(at) {
                *alpha = 1.0;
            }
            at += 1;
        }
        v
    };
    if let Some(m) = sel {
        let mask = crate::allocation::read_region(m, r, "applying the selection to copied pixels")?;
        let mk = m.format().channels().max(1);
        for (p, a) in px.chunks_exact_mut(n.max(1)).zip(mask.chunks_exact(mk)) {
            if let Some(alpha) = p.last_mut() {
                *alpha *= a[0];
            }
        }
    }
    Ok(px)
}

/// Merged composite of the visible document as a surface in the document's format.
fn merged_surface(doc: &Document) -> Surface {
    let fmt = doc.pixel_format();
    photocraft_compose::flatten_to_surface(doc, PixelFormat::new(fmt.mode, fmt.sample, true), None)
}

/// Estimated per-pixel effect-map storage and extra memory while building those maps.
#[derive(Clone, Copy, Default)]
struct MaskedCopyFxMemory {
    retained: u64,
    build: u64,
}

/// Additional retained stand-in tiles and conversion scratch for the split CMYK compositor.
#[derive(Clone, Copy, Default)]
struct MaskedCopyCmykSplitMemory {
    retained: u64,
    building: u64,
    enabled: bool,
}

/// Two RGB stand-ins are built for every allocated CMYK source tile, even outside the render
/// rectangle. The second is built while the first remains alive; converting up to one batch keeps
/// both input and converted f32 samples in flight. Their effect maps can also coexist.
fn masked_copy_cmyk_split_memory(tile_count: usize, source: PixelFormat) -> Option<MaskedCopyCmykSplitMemory> {
    let tiles = u64::try_from(tile_count).ok()?;
    let side = u64::try_from(photocraft_geom::TILE_SIZE).ok()?;
    let tile_pixels = side.checked_mul(side)?;
    let rgb = PixelFormat::new(ColorMode::Rgb, source.sample, source.alpha);
    let tile_bytes = tile_pixels.checked_mul(u64::try_from(rgb.bytes_per_pixel()).ok()?)?;
    // Include modest allowance for the tile maps and the list of source coordinates.
    let retained = tiles.checked_mul(tile_bytes.checked_add(256)?)?.checked_mul(2)?;
    let batch_tiles = photocraft_compose::BAND_PIXELS.div_ceil(tile_pixels).min(tiles);
    let batch_pixels = batch_tiles.checked_mul(tile_pixels)?;
    let float_channels = u64::try_from(source.channels()).ok()?.checked_add(u64::try_from(rgb.channels()).ok()?)?;
    let building = batch_pixels.checked_mul(float_channels.checked_mul(4)?)?;
    Some(MaskedCopyCmykSplitMemory { retained, building, enabled: true })
}

/// Match the compositor's split-ink branch without charging CMYK paths that do not use it.
fn masked_copy_uses_cmyk_split(doc: &Document) -> bool {
    doc.mode == ColorMode::Cmyk && !photocraft_compose::cmyk::supports(doc) && photocraft_compose::cmyk::renders_inks(doc)
}

/// Estimate the maps kept by a raster layer's enabled effects. Overlays reuse one shape map;
/// shadows/glows/satin keep one map each, bevel keeps two, and strokes share up to three distance
/// maps. Building a map may need additional temporary maps, and making the shape may hold a
/// full-region RGBA-f32 buffer and its alpha together. The builder can evolve independently,
/// so these allowances are deliberately a workload estimate, not a guaranteed upper bound.
fn masked_copy_effect_memory(layer: &Layer) -> Option<MaskedCopyFxMemory> {
    if !layer.effects.enabled {
        return Some(MaskedCopyFxMemory::default());
    }
    let mut maps = 1u64; // The shared shape map.
    let mut temp = 0u64;
    let mut any = false;
    let mut stroke = false;
    for effect in layer.effects.items.iter().filter(|effect| effect.enabled()) {
        any = true;
        match effect {
            photocraft_doc::Effect::DropShadow(_)
            | photocraft_doc::Effect::InnerShadow(_)
            | photocraft_doc::Effect::OuterGlow(_)
            | photocraft_doc::Effect::InnerGlow(_)
            | photocraft_doc::Effect::Satin(_) => {
                maps = maps.checked_add(1)?;
                temp = temp.max(16);
            }
            photocraft_doc::Effect::BevelEmboss(_) => {
                maps = maps.checked_add(2)?;
                temp = temp.max(32);
            }
            photocraft_doc::Effect::Stroke(_) => {
                stroke = true;
                temp = temp.max(32);
            }
            photocraft_doc::Effect::ColorOverlay { .. } | photocraft_doc::Effect::GradientOverlay { .. } | photocraft_doc::Effect::PatternOverlay { .. } => {}
        }
    }
    if !any {
        return Some(MaskedCopyFxMemory::default());
    }
    if stroke {
        maps = maps.checked_add(3)?;
    }
    let retained = maps.checked_mul(4)?;
    Some(MaskedCopyFxMemory { retained, build: retained.checked_add(temp)?.max(20) })
}

/// Estimated allocation workload for the new masked-layer bitmap.
///
/// The compositor holds destination tiles through the final prune, including transparent ones.
/// Band reserves are 48 bytes per RGB/gray pixel, 56 for Lab, and 64 for CMYK; they account for
/// the float composite/conversion buffers and, on native paths, source and mask buffers. Effect
/// maps can cover the entire effect region, not merely the current band. The CMYK split path
/// retains two stand-ins and two sets of effect maps. Map building, stand-in conversion, and band
/// rendering are estimated separately. This is a workload guard, not a proof of peak memory:
/// effects, prepared patterns, allocator overhead and other live documents may need more. The OS
/// export is a separate optional operation and is budgeted by the UI clipboard bridge.
fn masked_copy_estimate(area: Rect, fx_region: Rect, fmt: PixelFormat, effects: MaskedCopyFxMemory, split: MaskedCopyCmykSplitMemory) -> Option<u64> {
    let pixel_count = |r: Rect| u64::from(r.width()).checked_mul(u64::from(r.height()));
    let tile_side = i64::from(photocraft_geom::TILE_SIZE);
    let tile_count = |start: i32, end: i32| -> Option<u64> {
        if start >= end {
            return Some(0);
        }
        u64::try_from((i64::from(end) - 1).div_euclid(tile_side) - i64::from(start).div_euclid(tile_side) + 1).ok()
    };
    let tiles = tile_count(area.x0, area.x1)?.checked_mul(tile_count(area.y0, area.y1)?)?;
    let tile_pixels = u64::try_from(tile_side.checked_mul(tile_side)?).ok()?;
    let destination = tiles.checked_mul(tile_pixels)?.checked_mul(u64::try_from(fmt.bytes_per_pixel()).ok()?)?;
    let rows = u64::try_from(photocraft_compose::band_rows_for(area.width(), 0)).ok()?.min(u64::from(area.height()));
    let band_pixels = u64::from(area.width()).checked_mul(rows)?;
    let base_scratch = match fmt.mode {
        ColorMode::Cmyk => 64u64,
        ColorMode::Lab => 56u64,
        _ => 48u64,
    };
    let fx_pixels = pixel_count(fx_region)?;
    let resident = destination.checked_add(split.retained)?;
    let fx_render = effects.retained.checked_mul(if split.enabled { 2 } else { 1 })?;
    let fx_build = effects.build.checked_add(if split.enabled { effects.retained } else { 0 })?;
    let render_peak = resident.checked_add(band_pixels.checked_mul(base_scratch)?)?.checked_add(fx_pixels.checked_mul(fx_render)?)?;
    // The conversion Vec from an earlier band can remain allocated while a later band's effect
    // maps are rebuilt. Native CMYK needs one f32 for each channel, so count those bytes too.
    let conversion = band_pixels.checked_mul(u64::try_from(fmt.channels()).ok()?.checked_mul(4)?)?;
    let build_peak = resident.checked_add(conversion)?.checked_add(fx_pixels.checked_mul(fx_build)?)?;
    let standin_peak = resident.checked_add(split.building)?;
    Some(render_peak.max(build_peak).max(standin_peak))
}

/// Bound the estimated bitmap workload to half the configured document budget, capped at 1.5 GiB.
/// The remainder is intended to leave headroom; it is not a reservation or an available-RAM check.
/// The web preference already caps at 1.5 GiB. Actual memory use may exceed this estimate.
fn check_masked_copy_budget(
    area: Rect,
    fx_region: Rect,
    fmt: PixelFormat,
    effects: MaskedCopyFxMemory,
    split: MaskedCopyCmykSplitMemory,
    memory_mb: u32,
) -> Result<()> {
    let limit = (u64::from(memory_mb) * (1 << 20) / 2).min(3 << 29);
    let estimate = masked_copy_estimate(area, fx_region, fmt, effects, split);
    if estimate.is_none_or(|bytes| bytes > limit) {
        return Err(EngineError::Other(format!(
            "Could not copy the masked layer: its estimated bitmap working set exceeds the {limit}-byte copy memory limit; the clipboard was not changed"
        )));
    }
    Ok(())
}

fn copy(s: &mut Session, merged: bool, take_whole: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let clip = if merged {
        lift(&merged_surface(&d.doc), d.doc.selection.as_ref(), canvas)?
    } else {
        let id = d.active_layer.ok_or(EngineError::Other("no active layer".into()))?;
        let several = if take_whole { multi_copy_ids(d) } else { Vec::new() };
        let mut layers = if !take_whole {
            None
        } else if several.is_empty() {
            whole_layer(d, id)
        } else {
            whole_layers(d, &several)
        };
        if let Some(copies) = &mut layers {
            copy_linked_sources(&d.doc, copies)?;
        }
        let masked_raster = d.doc.layer(id).is_some_and(|l| matches!(l.content, LayerContent::Raster(_)) && (l.mask.is_some() || l.vector_mask.is_some()));
        match (d.doc.layer(id).and_then(Layer::surface).filter(|_| several.is_empty()), &layers) {
            // Preserve the editable layer separately while the OS clipboard and bitmap-only
            // targets receive what the masked layer actually shows. Its underlying colour
            // bounds still determine placement even when the mask hides every pixel.
            (Some(_), Some(copies)) if masked_raster => {
                let bounds = copies.layers.iter().filter_map(crate::layer_multi_cmds::layer_bounds).fold(Rect::EMPTY, |a, b| a.union(&b)).intersect(&canvas);
                let mut shown = (**copies).clone();
                for l in &mut shown.layers {
                    l.visible = true;
                }
                // Composite only where this isolated layer can draw. This includes its effects;
                // the compositor returns None for content that can draw anywhere, so such
                // cases safely fall back to the whole canvas.
                let area = shown.layers.first().and_then(|layer| photocraft_compose::composite_bounds(layer, canvas)).unwrap_or(canvas).intersect(&canvas);
                let fmt = shown.pixel_format();
                let fmt = PixelFormat::new(fmt.mode, fmt.sample, true);
                let (fx_region, effects) = if let Some(layer) = shown.layers.first() {
                    let effects = masked_copy_effect_memory(layer)
                        .ok_or_else(|| EngineError::Other("Could not estimate masked-layer effects; the clipboard was not changed".into()))?;
                    let margin = photocraft_compose::effects::margin(layer);
                    let region = photocraft_compose::effect_source_bounds(layer, canvas).inflate(margin).intersect(&canvas.inflate(margin));
                    (region, effects)
                } else {
                    (Rect::EMPTY, MaskedCopyFxMemory::default())
                };
                let split = if masked_copy_uses_cmyk_split(&shown) {
                    let source = shown
                        .layers
                        .first()
                        .and_then(Layer::surface)
                        .ok_or_else(|| EngineError::Other("Could not estimate CMYK clipboard pixels; the clipboard was not changed".into()))?;
                    masked_copy_cmyk_split_memory(source.tile_count(), source.format())
                        .ok_or_else(|| EngineError::Other("Could not estimate CMYK clipboard memory; the clipboard was not changed".into()))?
                } else {
                    MaskedCopyCmykSplitMemory::default()
                };
                check_masked_copy_budget(area, fx_region, fmt, effects, split, s.prefs().performance.effective_memory_usage_mb())?;
                crate::allocation::checkpoint("rendering the copied layer")?;
                let surface = photocraft_compose::composite_to_surface(&shown, area, fmt, None);
                // The compositor already returns an alpha surface limited to `area` inside the
                // canvas and prunes empty tiles. Re-lifting it would needlessly copy every edge
                // tile while keeping this surface alive (all tiles for a one-pixel-high layer).
                let bitmap_bounds = surface.content_bounds().intersect(&canvas);
                Clip { surface, layers, bounds: bounds.union(&bitmap_bounds), resolution: None, source: None }
            }
            (Some(surf), _) => Clip { layers, ..lift(surf, d.doc.selection.as_ref(), canvas)? },
            // A group or fill layer has no pixels of its own: the clipboard pixels are what it
            // shows. Adjustment layers show nothing by themselves, so their clip spans the canvas
            // they apply to: Paste places them as a full-canvas layer, and the UI still mirrors
            // the clip to the OS clipboard (an empty clip would let a stale OS image replace it).
            (None, Some(copies)) => {
                let mut shown = (**copies).clone();
                for l in &mut shown.layers {
                    l.visible = true;
                }
                let clip = lift(&merged_surface(&shown), None, canvas)?;
                let bounds = if clip.bounds.is_empty() { canvas } else { clip.bounds };
                Clip { layers, bounds, ..clip }
            }
            (None, None) => return Err(EngineError::Other("the active layer has no pixels".into())),
        }
    };
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let b = clip.bounds;
    let clip = Clip { resolution: Some(d.doc.resolution_dpi), source: Some(d.doc.id), ..clip };
    s.clipboard = Some(clip);
    Ok(json!({"bounds": [b.x0, b.y0, b.width(), b.height()]}))
}

/// Copy the targeted layer mask (`"target":"mask"`, #2928) or smart-filter mask
/// (`"target":"filterMask"`): the mask's values as a
/// grayscale clip, inside the selection or the whole canvas — an untouched mask copies at its
/// default value, so a plain Reveal All / Hide All mask transfers too. Paste then writes into
/// another layer's mask (targeted, #1035), in this or another document.
fn copy_mask(s: &mut Session, filter: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let id = d.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    let layer = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let mask = crate::mask_view_cmds::viewed_mask(layer, filter)
        .ok_or_else(|| EngineError::Other(if filter { "the active layer has no filter mask" } else { "the active layer has no layer mask" }.into()))?;
    let sel = d.doc.selection.as_ref();
    let area = sel.map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    if area.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let n = mask.surface.format().channels().max(1);
    let values = crate::allocation::read_region(&mask.surface, area, "copying the mask")?;
    let len = values
        .len()
        .checked_div(n)
        .and_then(|pixels| pixels.checked_mul(2))
        .ok_or_else(|| EngineError::Other("not enough memory for copying the mask (requested size overflowed); the document was not changed".into()))?;
    let cover = sel.map(|m| (m.read_region(area), m.format().channels().max(1)));
    let mut px = crate::allocation::filled(len, 0.0, "copying the mask")?;
    for (i, p) in values.chunks_exact(n).enumerate() {
        px[i * 2] = p.first().copied().unwrap_or(0.0);
        px[i * 2 + 1] = cover.as_ref().map_or(1.0, |(m, mk)| m.get(i * mk).copied().unwrap_or(0.0));
    }
    let fmt = PixelFormat::new(ColorMode::Grayscale, mask.surface.format().sample, true);
    let mut surface = Surface::new(fmt);
    surface
        .try_write_region(area, &px)
        .map_err(|e| EngineError::Other(format!("not enough memory while copying the mask ({e}); the document was not changed")))?;
    surface.prune();
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let b = bounds;
    let (resolution, source) = (Some(d.doc.resolution_dpi), Some(d.doc.id));
    s.clipboard = Some(Clip { surface, bounds: b, layers: None, resolution, source });
    Ok(json!({"bounds": [b.x0, b.y0, b.width(), b.height()], "target": if filter { "filterMask" } else { "mask" }}))
}

fn clear_selected(doc: &mut Document, id: LayerId, background: [f32; 4]) -> Result<()> {
    let sel = doc.selection.clone();
    let canvas = doc.bounds();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    clear_area(doc, id, area, sel.as_ref(), background)
}

/// Cut / Clear on a layer: makes the selected pixels transparent. The Background can't hold
/// transparency, so there the area is filled with the background colour instead (Photoshop).
pub(crate) fn clear_area(doc: &mut Document, id: LayerId, area: Rect, sel: Option<&Surface>, background: [f32; 4]) -> Result<()> {
    doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let bg = doc.is_background(id);
    let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
    if bg {
        crate::pixels::try_fill_surface(surf, area, background, sel, true)?;
    } else {
        crate::pixels::try_clear_surface(surf, area, sel)?;
    }
    surf.prune();
    Ok(())
}

/// Paste as a new layer (or into the targeted mask or channel: [`paste_to_target`]). `in_place` keeps the original position; otherwise the pixels are centred
/// on `center` (the view centre from the UI) or the canvas, unless they already overlap the canvas.
fn paste(s: &mut Session, p: &Value, in_place: bool) -> Result<Value> {
    crate::allocation::checkpoint("pasting pixels")?;
    let clip = clipboard_snapshot(s)?;
    let Some(d) = s.active() else {
        // Nothing open to paste into: the clipboard becomes a document of its own (#368).
        return new_from_clipboard(s);
    };
    if crate::channel_cmds::target_of(p) != crate::channel_cmds::Target::Pixels {
        return paste_to_target(s, p, in_place, None, "Paste");
    }
    // A targeted colour channel takes the paste instead of a new layer (#2307).
    if let Some(k) = crate::channel_clip::paste_color_target(s, p) {
        return crate::channel_clip::paste(s, p, k, in_place, None, "Paste");
    }
    let canvas = d.doc.bounds();
    let fmt = d.doc.pixel_format();
    let (dx, dy) = paste_offset(&clip, d.doc.id, canvas, p, in_place);
    if let Some(layers) =
        clip.layers.as_deref().filter(|_| crate::layer_copy_cmds::layered(d.doc.mode) && p.get("pixels").and_then(Value::as_bool) != Some(true))
    {
        return paste_layers(s, layers, dx, dy);
    }
    let moved = shifted(&clip.surface, dx, dy);
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let surf = if moved.format() == target { moved } else { moved.convert(target) };
    let id = s.edit("Paste", |doc, active| {
        let mut l = Layer::raster(doc.next_layer_name("Layer"), target);
        *crate::pixels_mut(&mut l)? = surf;
        let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::AutoEditable);
        *active = Some(id);
        doc.selection = None;
        Ok(id)
    })?;
    // `edit` reports only where the pasted pixels land as damage, not the whole document (#2901).
    Ok(json!({"layer": id.0, "offset": [dx, dy]}))
}

/// Paste of a whole copied layer ([`whole_layer`]): fresh copies (a group's children too),
/// converted to the document's colour, moved as the pixels would be, above the active layer. A pasted smart object gets its
/// own contents, independent of the copied one (as across documents).
fn paste_layers(s: &mut Session, layers: &Document, dx: i32, dy: i32) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    // Reject malformed clipboard trees before cloning or translating their recursively
    // nested layers; insertion is checked separately against the destination depth.
    crate::cmd::layer_multi_cmds::check_group_depth(layers, "Paste")?;
    let mut copies = layers.clone();
    for l in &mut copies.layers {
        *l = l.duplicate();
    }
    crate::layer_copy_cmds::adapt_copies(s, &mut copies, &d.doc)?;
    crate::layer_copy_cmds::translate_copies(&mut copies, &d.doc, dx, dy);
    if dx != 0 || dy != 0 {
        translate_unlinked_paste_masks(&mut copies.layers, dx, dy, 0)?;
    }
    let patterns: Vec<_> = copies.patterns.iter().filter(|pat| !d.doc.patterns.iter().any(|q| q.id == pat.id)).cloned().collect();
    // A copied smart object may name a PSD-embedded linked file; land its `lnk` block too (#3584).
    let mut linked = Vec::new();
    for (signature, key, data) in copies.metadata.psd_global_blocks.iter().filter(|(_, k, _)| matches!(k, b"lnk2" | b"lnk3" | b"lnkD" | b"lnkE")) {
        if d.doc.metadata.psd_global_blocks.iter().any(|(_, dk, dd)| dk == key && std::sync::Arc::ptr_eq(dd, data)) {
            continue;
        }
        // The clipboard may contain a subset of a destination block. Compare complete records,
        // so copying one layer in the same document does not duplicate large embedded files.
        let missing = photocraft_io::linked::select_items(data, &|_, raw| {
            !d.doc.metadata.psd_global_blocks.iter().any(|(_, dk, dd)| dk == key && photocraft_io::linked::contains_item(dd, raw))
        })
        .map_err(|e| EngineError::Other(format!("Could not paste smart-object contents: {e}; the document was not changed")))?;
        let missing = missing.map(std::sync::Arc::new).unwrap_or_else(|| data.clone());
        if !missing.is_empty() {
            linked.try_reserve(1).map_err(|_| EngineError::Other("not enough memory to paste smart-object source records".into()))?;
            linked.push((*signature, *key, missing));
        }
    }
    let id = s.edit("Paste", |doc, active| {
        doc.patterns.extend(patterns);
        doc.metadata.psd_global_blocks.extend(linked);
        // The first pasted layer goes where a new layer would; the rest stack above it.
        let mut place = crate::layer_place::Place::AutoEditable;
        for l in copies.layers {
            *active = Some(crate::layer_place::insert_new(doc, *active, l, place));
            place = crate::layer_place::Place::Above;
        }
        crate::cmd::layer_multi_cmds::check_group_depth(doc, "Paste")?;
        doc.selection = None;
        active.ok_or(EngineError::Other("nothing to paste".into()))
    })?;
    Ok(json!({"layer": id.0, "offset": [dx, dy]}))
}

/// Paste moves an entire copied layer, including unlinked masks. Ordinary layer movement
/// still respects the link flag. Walk the copies directly so groups need no ID lookup table.
fn translate_unlinked_paste_masks(layers: &mut [Layer], dx: i32, dy: i32, depth: usize) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(EngineError::Other("can't paste masks from layers nested too deeply".into()));
    }
    for layer in layers {
        if let Some(mask) = &mut layer.mask
            && !mask.linked
        {
            mask.surface = shifted(&mask.surface, dx, dy);
        }
        if let Some(mask) = &mut layer.vector_mask
            && !mask.linked
        {
            mask.path = mask.path.transform(&photocraft_geom::Affine::translate(f64::from(dx), f64::from(dy)));
        }
        if let Some(children) = layer.children_mut().filter(|children| !children.is_empty()) {
            translate_unlinked_paste_masks(children, dx, dy, depth + 1)?;
        }
    }
    Ok(())
}

/// How far a paste moves the clipboard pixels: not at all in place (or when they lie on the
/// canvas and no `center` is given), else onto `center` (the view centre from the UI) or the
/// canvas centre, then kept on the canvas as Photoshop keeps it: pixels that fit stay wholly
/// inside, larger ones start at the canvas's top-left corner (#6759). `"clampToCanvas": false`
/// skips that, for Paste Into / Outside, which centre on the selection.
pub(crate) fn paste_offset(clip: &Clip, doc: DocId, canvas: Rect, p: &Value, in_place: bool) -> (i32, i32) {
    let home = clip.source == Some(doc);
    if in_place || (home && clip.bounds.intersect(&canvas) == clip.bounds && p.get("center").is_none()) {
        return (0, 0);
    }
    let c = p.get("center").and_then(Value::as_array).filter(|a| a.len() >= 2).map(|a| (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)));
    let (cx, cy) = c.unwrap_or((mid(canvas.x0, canvas.x1), mid(canvas.y0, canvas.y1)));
    let b = clip.bounds;
    let (dx, dy) = ((cx - mid(b.x0, b.x1)).round(), (cy - mid(b.y0, b.y1)).round());
    if p.get("clampToCanvas").and_then(Value::as_bool) == Some(false) || b.is_empty() {
        return (dx as i32, dy as i32);
    }
    (clamp_shift(dx, b.x0, b.x1, canvas.x0, canvas.x1), clamp_shift(dy, b.y0, b.y1, canvas.y0, canvas.y1))
}

/// The midpoint of `a..b`, each edge converted before adding so extreme coordinates can't overflow.
fn mid(a: i32, b: i32) -> f64 {
    (f64::from(a) + f64::from(b)) / 2.0
}

/// Shift `d` of the span `b0..b1` along one axis, changed as little as needed to keep the span
/// inside `c0..c1` when it fits; a span longer than the canvas starts at `c0`.
fn clamp_shift(d: f64, b0: i32, b1: i32, c0: i32, c1: i32) -> i32 {
    let (b0, b1, c0, c1) = (i64::from(b0), i64::from(b1), i64::from(c0), i64::from(c1));
    // `as` saturates a NaN or out-of-range shift, then the clamp brings it into range.
    let start = if b1 - b0 <= c1 - c0 { b0.saturating_add(d as i64).clamp(c0, c1 - (b1 - b0)) } else { c0 };
    i32::try_from(start - b0).unwrap_or(if start > b0 { i32::MAX } else { i32::MIN })
}

pub(crate) fn shifted(surface: &Surface, dx: i32, dy: i32) -> Surface {
    if dx == 0 && dy == 0 { surface.clone() } else { photocraft_algo::resample::translate_surface(surface, dx, dy) }
}

/// Paste into the targeted layer mask, alpha channel or Quick Mask (`"target"`) instead of as a
/// new layer (Photoshop, #1035): the pasted pixels' luminosity, placed as a paste places them,
/// over what was there. Their transparency, and `limit` (Paste Into / Outside), let it show
/// through. One history step.
pub(crate) fn paste_to_target(s: &mut Session, p: &Value, in_place: bool, limit: Option<&Surface>, label: &str) -> Result<Value> {
    crate::allocation::checkpoint("pasting pixels")?;
    let clip = clipboard_snapshot(s)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let layer = d.active_layer;
    let (dx, dy) = paste_offset(&clip, d.doc.id, canvas, p, in_place);
    let src = shifted(&clip.surface, dx, dy);
    let area = src.content_bounds().intersect(&canvas);
    s.edit(label, |doc, _| {
        // A spot channel stores ink: the paste goes over its tone, then back to ink (#5787).
        let spot = crate::channel_cmds::is_spot_target(doc, p);
        let (surf, _) = crate::channel_cmds::target_surface(doc, layer, p)?;
        if spot {
            crate::channel_cmds::invert_region(surf, area)?;
        }
        crate::fill_cmds::composite_over(surf, &src, area, limit)?;
        if spot {
            crate::channel_cmds::invert_region(surf, area)?;
        }
        surf.prune();
        doc.selection = None;
        Ok(())
    })?;
    Ok(json!({"target": p.get("target"), "offset": [dx, dy]}))
}

/// A new document the size of the clipboard image, holding it as its one layer, in the pixel
/// format it was copied in (#368).
fn new_from_clipboard(s: &mut Session) -> Result<Value> {
    let clip = clipboard_snapshot(s)?;
    let b = clip.bounds;
    let (w, h) = (b.width(), b.height());
    if w == 0 || h == 0 {
        return Err(EngineError::Other("the clipboard image has no size".into()));
    }
    let (Some(dx), Some(dy)) = (b.x0.checked_neg(), b.y0.checked_neg()) else {
        return Err(EngineError::Other("the clipboard origin is outside the coordinate range for a new document".into()));
    };
    let fmt = clip.surface.format();
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let moved = if dx == 0 && dy == 0 { clip.surface } else { photocraft_algo::resample::translate_surface(&clip.surface, dx, dy) };
    let mut doc = Document::new("Untitled", photocraft_geom::Size::new(w, h), fmt.mode, fmt.sample);
    if let Some(r) = clip.resolution.filter(|r| r.is_finite() && *r > 0.0) {
        doc.resolution_dpi = r;
    }
    let mut l = Layer::raster(doc.next_layer_name("Layer"), target);
    *crate::pixels_mut(&mut l)? = if moved.format() == target { moved } else { moved.convert(target) };
    doc.layers.push(l);
    let i = s.add_document(doc, None);
    // Clipboard content has never been saved; closing this tab must ask before discarding it.
    if let Some(st) = s.active_mut() {
        st.saved_revision = 0;
    }
    Ok(json!({"document": i, "width": w, "height": h}))
}

fn layer_via(s: &mut Session, cut: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = active_id(s)?;
    let has_sel = d.doc.selection.is_some();
    if !has_sel && !cut && crate::layer_multi_cmds::multi(s, &Value::Null) {
        // No selection, several layers selected: copy every one of them (Photoshop, #2777).
        return crate::layer_multi_cmds::duplicate_selected(s, true, "Layer Via Copy");
    }
    if !has_sel && !cut {
        // No selection: Layer via Copy duplicates the whole layer (Photoshop).
        let nid = s.edit("Layer Via Copy", |doc, active| {
            let src = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let mut dup = src.duplicate();
            dup.name = doc.copy_name(&src.name);
            dup.locks = Default::default();
            let nid = doc.insert_above(Some(id), dup);
            *active = Some(nid);
            Ok(nid)
        })?;
        return Ok(json!({"layer": nid.0}));
    }
    let surf = d.doc.layer(id).and_then(Layer::surface).ok_or(EngineError::Other("the active layer has no pixels".into()))?;
    let clip = lift(surf, d.doc.selection.as_ref(), d.doc.bounds())?;
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("Could not complete the command: the selected area is empty".into()));
    }
    let bg = s.tools.background;
    let label = if cut { "Layer Via Cut" } else { "Layer Via Copy" };
    let nid = s.edit(label, |doc, active| {
        if cut {
            clear_selected(doc, id, bg)?;
        }
        let mut l = Layer::raster(doc.next_layer_name("Layer"), clip.surface.format());
        *crate::pixels_mut(&mut l)? = clip.surface;
        let nid = crate::layer_place::insert_new(doc, Some(id), l, crate::layer_place::Place::Above);
        *active = Some(nid);
        // Photoshop deselects: a following Free Transform moves the whole new layer (#1096).
        doc.selection = None;
        Ok(nid)
    })?;
    Ok(json!({"layer": nid.0}))
}

fn merge_visible(s: &mut Session) -> Result<Value> {
    s.edit("Merge Visible", |doc, active| {
        let visible: Vec<usize> = doc.layers.iter().enumerate().filter(|(_, l)| l.visible).map(|(i, _)| i).collect();
        if visible.len() < 2 && !visible.iter().any(|i| doc.layers[*i].is_group()) {
            return Err(EngineError::Other("there are not enough visible layers to merge".into()));
        }
        let lowest = visible[0];
        let base = doc.layers[lowest].clone();
        let is_background = lowest == 0 && doc.is_background(base.id);
        // Composite of visible layers only (hidden ones stay where they are).
        let mut solo = doc.clone();
        solo.layers.retain(|l| l.visible);
        let fmt = doc.pixel_format();
        let fmt = if is_background { fmt } else { PixelFormat::new(fmt.mode, fmt.sample, true) };
        let mut merged = Layer::raster(base.name.clone(), fmt);
        merged.locks = base.locks;
        *crate::pixels_mut(&mut merged)? = crate::pixels::composite_layers(&solo, fmt, is_background.then_some([1.0, 1.0, 1.0]));
        let mid = merged.id;
        let mut out = Vec::with_capacity(doc.layers.len());
        for (i, l) in doc.layers.drain(..).enumerate() {
            if i == lowest {
                out.push(merged.clone());
            } else if !l.visible {
                out.push(l);
            }
        }
        doc.layers = out;
        *active = Some(mid);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Histogram of a pixel layer (8-bit bins per channel: R, G, B) inside the selection.
fn histograms(surf: &Surface, area: Rect) -> [[u64; 256]; 3] {
    let mut h = [[0u64; 256]; 3];
    let step = ((area.width() as u64 * area.height() as u64) / 4_000_000).max(1) as usize; // sample big images
    let mut row = vec![[0u8; 4]; area.width() as usize];
    for y in (area.y0..area.y1).step_by(step.isqrt().max(1)) {
        surf.read_rgba8_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
        for p in row.iter().step_by(step.isqrt().max(1)) {
            if p[3] == 0 {
                continue;
            }
            for c in 0..3 {
                h[c][p[c] as usize] += 1;
            }
        }
    }
    h
}

/// Black and white points with `clip` (fraction) clipped at each end.
fn clip_points(h: &[u64; 256], clip: f64) -> (f32, f32) {
    let total: u64 = h.iter().sum();
    if total == 0 {
        return (0.0, 1.0);
    }
    let lim = (total as f64 * clip) as u64;
    let (mut acc, mut lo) = (0u64, 0usize);
    while lo < 254 && acc + h[lo] <= lim {
        acc += h[lo];
        lo += 1;
    }
    let (mut acc, mut hi) = (0u64, 255usize);
    while hi > lo + 1 && acc + h[hi] <= lim {
        acc += h[hi];
        hi -= 1;
    }
    (lo as f32 / 255.0, hi as f32 / 255.0)
}

fn median(h: &[u64; 256]) -> f32 {
    let total: u64 = h.iter().sum();
    let mut acc = 0;
    for (i, v) in h.iter().enumerate() {
        acc += v;
        if acc * 2 >= total {
            return i as f32 / 255.0;
        }
    }
    0.5
}

/// Image › Auto Tone / Auto Contrast / Auto Color (0.1% clipping, Photoshop's defaults).
fn auto_adjust(s: &mut Session, kind: &str) -> Result<Value> {
    let id = active_id(s)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let surf = d.doc.layer(id).and_then(Layer::surface).ok_or(EngineError::Other("the active layer has no pixels".into()))?;
    let area = d.doc.selection.as_ref().map_or(d.doc.bounds(), |m| m.content_bounds()).intersect(&d.doc.bounds());
    let h = histograms(surf, area);
    let clip = 0.001;
    let mut per = [LevelsChannel::default(), LevelsChannel::default(), LevelsChannel::default()];
    match kind {
        "contrast" => {
            // Same points for all channels (preserves colour), from the combined histogram.
            let mut all = [0u64; 256];
            for c in &h {
                for (a, v) in all.iter_mut().zip(c) {
                    *a += v;
                }
            }
            let (lo, hi) = clip_points(&all, clip);
            for p in &mut per {
                (p.in_black, p.in_white) = (lo, hi);
            }
        }
        _ => {
            for (c, p) in per.iter_mut().enumerate() {
                (p.in_black, p.in_white) = clip_points(&h[c], clip);
            }
            if kind == "color" {
                // Neutralise midtones: move each channel's median onto the average median.
                let meds: Vec<f32> = (0..3).map(|c| median(&h[c])).collect();
                let target = meds.iter().sum::<f32>() / 3.0;
                for (c, p) in per.iter_mut().enumerate() {
                    let t = ((meds[c] - p.in_black) / (p.in_white - p.in_black).max(1e-3)).clamp(0.02, 0.98);
                    let want = ((target - p.in_black) / (p.in_white - p.in_black).max(1e-3)).clamp(0.02, 0.98);
                    // out = t^(1/g) = want  →  g = ln t / ln want
                    p.gamma = (t.ln() / want.ln()).clamp(0.1, 9.99);
                }
            }
        }
    }
    let adj = Adjustment::Levels { master: LevelsChannel::default(), per_channel: per, space: Default::default(), black: LevelsChannel::default() };
    let label = match kind {
        "contrast" => "Auto Contrast",
        "color" => "Auto Color",
        _ => "Auto Tone",
    };
    s.edit(label, |doc, _| {
        let sel = doc.selection.clone();
        let mode = doc.mode;
        let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
        crate::pixels::adjust_surface(surf, &adj, sel.as_ref(), mode).map_err(EngineError::Other)?;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn toggle_last_state(s: &mut Session) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let redo = st.history.can_redo();
    Ok(json!(if redo { s.redo() } else { s.undo() }))
}

/// Edit › Transform › Again: replay the last `edit.transform` on the selected layers (#6501).
fn transform_again(s: &mut Session) -> Result<Value> {
    let ids = s.active().map(crate::transform_cmds::selection_targets).unwrap_or_default();
    let p = transform_again_params(s, &ids)?;
    s.execute("edit.transform", p)
}

/// Transform Again on a copy (⌥⇧⌘T, no menu item): duplicates the active layer and repeats the
/// last transform on the copy, as one history step, so pressing it again steps and repeats
/// (#352). The transform is worked out before anything changes, so a refusal leaves no copy.
fn transform_again_copy(s: &mut Session) -> Result<Value> {
    let id = active_id(s)?;
    let p = transform_again_params(s, &[id])?;
    let journal = s.journal.len();
    // The steps are recorded (Again on a Copy is not replayable itself): with one layer selected,
    // "duplicate the current layer" plays back on any document, a layer id only on this one.
    let dup = if crate::layer_multi_cmds::multi(s, &Value::Null) { json!({"layer": id.0}) } else { json!({}) };
    s.execute("layer.duplicate", dup)?;
    match s.execute("edit.transform", p) {
        Ok(r) => {
            let st = s.active_mut().ok_or(EngineError::NoDocument)?;
            st.history.purge_last();
            st.history.set_current_label("Transform Again");
            Ok(r)
        }
        Err(e) => {
            // The copy (e.g. of a layer whose position is locked) couldn't be transformed: take
            // it back, leaving nothing to redo and no Duplicate step to record.
            s.rollback();
            s.journal.truncate(journal);
            Err(e)
        }
    }
}

/// The `edit.transform` params that repeat the last transform on the layers `ids` (their frame
/// together is the box it starts from).
fn transform_again_params(s: &Session, ids: &[LayerId]) -> Result<Value> {
    let (_, last) =
        s.journal.iter().rev().find(|(id, _)| id == "edit.transform").cloned().ok_or(EngineError::Other("there is no transform to repeat".into()))?;
    let (rect, quad) = (last.get("rect").cloned(), last.get("quad").cloned());
    let mut p = json!({"interpolation": last.get("interpolation").cloned().unwrap_or(json!("bicubic"))});
    match (rect, quad, last.get("matrix").cloned()) {
        (Some(r), Some(q), _) => {
            let v = |x: &Value| -> Vec<f64> { x.as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default() };
            let r = v(&r);
            let q: Vec<Vec<f64>> = q.as_array().map(|a| a.iter().map(v).collect()).unwrap_or_default();
            if r.len() != 4 || q.len() != 4 || q.iter().any(|row| row.len() < 2) {
                return Err(EngineError::Other("the last transform can't be repeated".into()));
            }
            let h = photocraft_algo::transform::Homography::rect_to_quad(
                [r[0], r[1], r[2], r[3]],
                [[q[0][0], q[0][1]], [q[1][0], q[1][1]], [q[2][0], q[2][1]], [q[3][0], q[3][1]]],
            )
            .ok_or(EngineError::Other("the last transform can't be repeated".into()))?;
            let d = s.active().ok_or(EngineError::NoDocument)?;
            d.doc.layer(active_id(s)?).ok_or(EngineError::Other("no active layer".into()))?;
            let b = crate::transform_cmds::transform_bounds_of(&d.doc, ids);
            if b.is_empty() {
                return Err(EngineError::Other("nothing to transform".into()));
            }
            let nr = [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64];
            let corners = [(nr[0], nr[1]), (nr[2], nr[1]), (nr[2], nr[3]), (nr[0], nr[3])].map(|(x, y)| {
                let (a, b) = h.apply(x, y);
                [a, b]
            });
            p["rect"] = json!(nr);
            p["quad"] = json!(corners);
        }
        (_, _, Some(m)) => p["matrix"] = m,
        _ => return Err(EngineError::Other("the last transform can't be repeated".into())),
    }
    Ok(p)
}

/// Guides further than this from the canvas origin are refused: far past any document (300000 px)
/// yet well within f32 precision.
const MAX_GUIDE_PX: f64 = 1_000_000.0;

/// A guide `position` in document pixels: a number of pixels, or text with a unit as in Photoshop's
/// New Guide dialog ("50%", "2.5 cm", "12 pt"). Percent is of the document's width for a vertical
/// guide and its height for a horizontal one; points and picas follow Units & Rulers › Point Size.
/// `None` when there is no `position`.
fn guide_position(s: &Session, p: &Value, vertical: bool) -> Result<Option<f64>> {
    let Some(v) = p.get("position") else { return Ok(None) };
    let px = match (v.as_f64(), v.as_str()) {
        (Some(n), _) => n,
        (None, Some(text)) => {
            let (n, unit) = crate::prefs::Unit::parse_length(text).ok_or_else(|| {
                EngineError::Other(format!("`position` {text:?} is not a length: give a number of pixels, or a number with px, in, cm, mm, pt, pc or %"))
            })?;
            let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
            let extent = if vertical { doc.size.width } else { doc.size.height };
            unit.unwrap_or(crate::prefs::Unit::Pixels).to_px(n, doc.resolution_dpi as f64, extent as f64, s.prefs().units_and_rulers.point_size.per_inch())
        }
        _ => return Err(EngineError::Other("`position` must be a number of pixels or text with a unit".into())),
    };
    if !px.is_finite() || px.abs() > MAX_GUIDE_PX {
        return Err(EngineError::Other(format!("`position` is out of range (at most {MAX_GUIDE_PX} px from the canvas origin)")));
    }
    Ok(Some(px))
}

/// Photoshop's guide colours, in its order, by the names `view.newGuide` takes: New Guide's and
/// New Guide Layout's Color lists. Cyan (the default guide colour in Preferences, so a guide
/// dragged from a ruler reads as Cyan), Magenta and Yellow are the values New Guide Layout first
/// shipped with; the rest approximate Photoshop's on screen, rounded to web-safe steps.
pub const GUIDE_COLORS: [(&str, [u8; 3]); 9] = [
    ("lightBlue", [0x33, 0x66, 0xff]),
    ("lightRed", [0xff, 0x33, 0x33]),
    ("green", [0x33, 0xff, 0x33]),
    ("mediumBlue", [0x33, 0x33, 0xff]),
    ("yellow", [0xff, 0xcc, 0x00]),
    ("magenta", [0xff, 0x00, 0xff]),
    ("cyan", [0x4a, 0xff, 0xff]),
    ("lightGray", [0xe7, 0xe7, 0xe7]),
    ("black", [0x00, 0x00, 0x00]),
];

/// A guide `color`: one of [`GUIDE_COLORS`]' names or `"#rrggbb"`. `None` when there is no `color`
/// (the guide uses Preferences › Guides, Grid & Slices › Guide colour).
fn guide_color(p: &Value) -> Result<Option<[u8; 3]>> {
    let Some(v) = p.get("color") else { return Ok(None) };
    let text = v.as_str().ok_or(EngineError::Other("`color` must be a colour name or \"#rrggbb\"".into()))?;
    GUIDE_COLORS.iter().find(|(name, ..)| name.eq_ignore_ascii_case(text)).map(|(.., rgb)| *rgb).or_else(|| crate::prefs::parse_hex(text)).map(Some).ok_or_else(
        || EngineError::Other(format!("`color` {text:?} is not a guide colour: use \"#rrggbb\" or one of {}", GUIDE_COLORS.map(|c| c.0).join(", "))),
    )
}

/// Edit Selected Guides (the guide context menu): change one guide's orientation, position and
/// colour in one history step. A guide that keeps its orientation keeps its index; one that turns
/// goes to the end of the other list. Without `color` it keeps its own colour, or none.
fn edit_guide(s: &mut Session, p: &Value) -> Result<Value> {
    let orientation = |key: &str| match p.get(key).map(|v| v.as_str()) {
        None => Ok(None),
        Some(Some("horizontal")) => Ok(Some(false)),
        Some(Some("vertical")) => Ok(Some(true)),
        Some(_) => Err(EngineError::Other(format!("`{key}` must be \"horizontal\" or \"vertical\""))),
    };
    let from = orientation("orientation")?.unwrap_or(false);
    let to = orientation("newOrientation")?.unwrap_or(from);
    let index = p.get("index").and_then(Value::as_u64).ok_or(EngineError::Other("missing `index`".into()))?;
    let pos = guide_position(s, p, to)?;
    let color = guide_color(p)?;
    s.edit("Edit Guide", |doc, _| {
        let guides = &mut doc.guides;
        let list = if from { &guides.vertical } else { &guides.horizontal };
        let i = usize::try_from(index).ok().filter(|i| *i < list.len()).ok_or(EngineError::Other("no such guide".into()))?;
        let old = list[i];
        let at = pos.unwrap_or(old);
        let rgb = color.or_else(|| guides.color(from, old));
        if from == to {
            let list = if from { &mut guides.vertical } else { &mut guides.horizontal };
            list[i] = at;
        } else {
            (if from { &mut guides.vertical } else { &mut guides.horizontal }).remove(i);
            (if to { &mut guides.vertical } else { &mut guides.horizontal }).push(at);
        }
        guides.prune_colors();
        if rgb.is_some() {
            guides.set_color(to, at, rgb);
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// View › New Guide / guide moves (undoable, like Photoshop's "New Guide"/"Move Guide" states).
fn guide_cmd(s: &mut Session, p: &Value, op: &str) -> Result<Value> {
    let vertical = p.get("orientation").and_then(Value::as_str).unwrap_or("horizontal") == "vertical";
    // Move Guide can turn the guide (⌥ while dragging it, as in Photoshop): `toOrientation`.
    let to_vertical = match p.get("toOrientation").map(|v| v.as_str()) {
        None => vertical,
        Some(Some("vertical")) => true,
        Some(Some("horizontal")) => false,
        Some(_) => return Err(EngineError::Other("`toOrientation` must be \"horizontal\" or \"vertical\"".into())),
    };
    if to_vertical != vertical && op != "move" {
        return Err(EngineError::Other("`toOrientation` applies to view.moveGuide only".into()));
    }
    // A % position is of the axis the guide ends up on.
    let pos = guide_position(s, p, to_vertical)?;
    let color = if op == "new" { guide_color(p)? } else { None };
    let index = p.get("index").and_then(Value::as_u64).map(|v| v as usize);
    let label = match op {
        "new" => "New Guide",
        "move" => "Move Guide",
        "delete" => "Delete Guide",
        _ => "Clear Guides",
    };
    s.edit(label, |doc, _| {
        let guides = &mut doc.guides;
        let g = if vertical { &mut guides.vertical } else { &mut guides.horizontal };
        match op {
            "new" => {
                let at = pos.ok_or(EngineError::Other("missing `position`".into()))?;
                g.push(at);
                if color.is_some() {
                    guides.set_color(vertical, at, color);
                }
            }
            "move" => {
                let i = index.filter(|i| *i < g.len()).ok_or(EngineError::Other("no such guide".into()))?;
                let to = pos.ok_or(EngineError::Other("missing `position`".into()))?;
                let from = if to_vertical == vertical {
                    std::mem::replace(&mut g[i], to)
                } else {
                    let from = g.remove(i);
                    if to_vertical {
                        guides.vertical.push(to)
                    } else {
                        guides.horizontal.push(to)
                    }
                    from
                };
                // The guide takes its colour along; another guide left at the old position keeps it.
                if let Some(rgb) = guides.color(vertical, from) {
                    guides.set_color(to_vertical, to, Some(rgb));
                }
                guides.prune_colors();
            }
            "delete" => {
                let i = index.filter(|i| *i < g.len()).ok_or(EngineError::Other("no such guide".into()))?;
                g.remove(i);
                guides.prune_colors();
            }
            _ => {
                guides.horizontal.clear();
                guides.vertical.clear();
                guides.colors.clear();
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Farthest the ruler zero point may sit from the image, in document pixels.
const MAX_RULER_ORIGIN: f64 = 1.0e7;

/// Move the rulers' zero point (`view.setRulerOrigin`), or put it back at the image's top-left
/// (`view.resetRulerOrigin`, `p` = None). Undoable like the guides.
fn ruler_origin_cmd(s: &mut Session, p: Option<&Value>) -> Result<Value> {
    let origin = match p {
        None => [0.0, 0.0],
        Some(p) => {
            let axis = |k: &str| {
                p.get(k)
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite() && v.abs() <= MAX_RULER_ORIGIN)
                    .ok_or_else(|| EngineError::Other(format!("`{k}` must be a number of pixels within ±{MAX_RULER_ORIGIN}")))
            };
            [axis("x")?, axis("y")?]
        }
    };
    let label = if p.is_some() { "Set Ruler Origin" } else { "Reset Ruler Origin" };
    s.edit(label, |doc, _| {
        doc.ruler_origin = origin;
        Ok(())
    })?;
    Ok(json!({"x": origin[0], "y": origin[1]}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!("edit.cut", "Cut", &["Edit"], Some("Cmd+X"), "{} (a targeted colour channel cuts that channel only)", has_pixels, |s, p| {
            // Refuse a locked layer before copying, so a refused Cut leaves the clipboard alone.
            let id = active_id(s)?;
            crate::commands::check_pixels_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
            // A targeted colour channel cuts that channel only (#2698).
            if let Some(k) = crate::channel_clip::paste_color_target(s, p) {
                return crate::channel_clip::cut(s, id, k);
            }
            let r = copy(s, false, false)?;
            let bg = s.tools.background;
            s.edit("Cut Pixels", |doc, _| clear_selected(doc, id, bg))?;
            Ok(r)
        }),
        spec!(
            "edit.copy",
            "Copy",
            &["Edit"],
            Some("Cmd+C"),
            r##"{"target":"mask|filterMask"? = the targeted mask, copied as grayscale (#2928)} (a targeted colour or alpha channel copies that channel, as grayscale)"##,
            can_copy,
            |s, p| {
                if crate::commands::is_mask_target(p) {
                    return copy_mask(s, crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::FilterMask);
                }
                match crate::channel_clip::copy(s)? {
                    Some(r) => Ok(r),
                    None => copy(s, false, true),
                }
            }
        ),
        spec!("edit.copyMerged", "Copy Merged", &["Edit"], Some("Cmd+Shift+C"), "{}", has_doc, |s, _| copy(s, true, false)),
        spec!(
            "edit.paste",
            "Paste",
            &["Edit"],
            Some("Cmd+V"),
            concat!(
                r#"{"center":[x,y]? (view centre; default keeps the position when it overlaps the canvas),"clampToCanvas":bool=true (keep the paste on the canvas, or at its top-left when larger),"pixels":bool=false (paste the clipboard bitmap only),"#,
                paste_target!(),
                "} (with no document open: a new document from the clipboard)"
            ),
            has_clip_only,
            |s, p| paste(s, p, false)
        ),
        spec!(
            "file.newFromClipboard",
            "New from Clipboard",
            &["File"],
            None,
            "{} (a new document the size of the clipboard image, holding it as one layer)",
            has_clip_only,
            |s, _| new_from_clipboard(s)
        ),
        spec!(
            "edit.pasteSpecial.pasteInPlace",
            "Paste in Place",
            &["Edit", "Paste Special"],
            Some("Cmd+Shift+V"),
            concat!("{", paste_target!(), "}"),
            has_clip,
            |s, p| paste(s, p, true)
        ),
        spec!("layer.new.layerViaCopy", "Layer via Copy", &["Layer", "New"], Some("Cmd+J"), "{}", has_doc, |s, _| layer_via(s, false)),
        spec!("layer.new.layerViaCut", "Layer via Cut", &["Layer", "New"], Some("Cmd+Shift+J"), "{}", has_pixels, |s, _| layer_via(s, true)),
        spec!("layer.mergeVisible", "Merge Visible", &["Layer"], Some("Cmd+Shift+E"), "{}", has_doc, |s, _| merge_visible(s)),
        spec!("image.autoTone", "Auto Tone", &["Image"], Some("Cmd+Shift+L"), "{}", has_pixels, |s, _| auto_adjust(s, "tone")),
        spec!("image.autoContrast", "Auto Contrast", &["Image"], Some("Cmd+Alt+Shift+L"), "{}", has_pixels, |s, _| auto_adjust(s, "contrast")),
        spec!("image.autoColor", "Auto Color", &["Image"], Some("Cmd+Shift+B"), "{}", has_pixels, |s, _| auto_adjust(s, "color")),
        spec!("edit.toggleLastState", "Toggle Last State", &["Edit"], Some("Cmd+Alt+Z"), "{}", has_doc, |s, _| toggle_last_state(s)),
        spec!("edit.transform.again", "Again", &["Edit", "Transform"], Some("Cmd+Shift+T"), "{}", has_doc, |s, _| transform_again(s)),
        spec!(
            "edit.transform.againCopy",
            "Transform Again on a Copy",
            &[],
            Some("Cmd+Alt+Shift+T"),
            "{} (duplicates the active layer and repeats the last transform on the copy: step and repeat)",
            has_doc,
            |s, _| transform_again_copy(s)
        ),
        spec!(
            "view.newGuide",
            "New Guide…",
            &["View"],
            None,
            r##"{"orientation":"horizontal|vertical","position":px or "<number><unit>","color":"lightBlue|lightRed|green|mediumBlue|yellow|magenta|cyan|lightGray|black" or "#rrggbb"?} (units px, in, cm, mm, pt, pc or %; % is of the width for a vertical guide, the height for a horizontal one; without a color the guide uses the guide colour in Preferences)"##,
            has_doc,
            |s, p| guide_cmd(s, p, "new")
        ),
        spec!(
            "view.moveGuide",
            "Move Guide",
            &[],
            None,
            r##"{"orientation":"horizontal|vertical","index":n,"position":px or "<number><unit>","toOrientation":"horizontal|vertical"?} (units as for view.newGuide; toOrientation turns the guide, its position then on the new axis)"##,
            has_doc,
            |s, p| guide_cmd(s, p, "move")
        ),
        spec!(
            "view.editGuide",
            "Edit Guide",
            &[],
            None,
            r##"{"orientation":"horizontal|vertical","index":n,"newOrientation":"horizontal|vertical"?,"position":px or "<number><unit>"?,"color":name or "#rrggbb"?} (Edit Selected Guides: units and colours as for view.newGuide; % is of the new orientation's extent; whatever is left out stays as it was)"##,
            has_doc,
            edit_guide
        ),
        spec!("view.deleteGuide", "Delete Guide", &[], None, r##"{"orientation":"horizontal|vertical","index":n}"##, has_doc, |s, p| guide_cmd(s, p, "delete")),
        spec!("view.clearGuides", "Clear Guides", &["View"], None, "{}", has_doc, |s, p| guide_cmd(s, p, "clear")),
        spec!(
            "view.setRulerOrigin",
            "Set Ruler Origin",
            &[],
            None,
            r##"{"x":px,"y":px} (the rulers' zero point in document pixels; the rulers and Info panel measure from it)"##,
            has_doc,
            |s, p| ruler_origin_cmd(s, Some(p))
        ),
        spec!("view.resetRulerOrigin", "Reset Ruler Origin", &[], None, "{} (zero point back at the image's top-left)", has_doc, |s, _| ruler_origin_cmd(
            s, None
        )),
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod clipboard_tests;
