//! Everyday Photoshop commands: clipboard (Cut/Copy/Copy Merged/Paste/Paste in Place), Layer via
//! Copy/Cut, Merge Visible, Auto Tone/Contrast/Color, Toggle Last State and Transform Again.

use photocraft_color::PixelFormat;
use photocraft_doc::adjust::LevelsChannel;
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, LayerId};
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

/// Clipboard bitmap and origin, with an optional independent, editable layer snapshot.
#[derive(Clone, Debug)]
pub struct Clip {
    pub surface: Surface,
    pub bounds: Rect,
    pub layers: Option<Document>,
    pub profile: Option<std::sync::Arc<photocraft_cms::Profile>>,
}

impl Clip {
    /// External bitmap / selected pixels: never contains an editable layer or its masks.
    pub fn pixels(surface: Surface, bounds: Rect) -> Self {
        Self { surface, bounds, layers: None, profile: None }
    }

    /// Validate the native RGBA8 bridge before allocating or interpreting external bytes.
    pub fn bitmap_len(w: u32, h: u32) -> Option<usize> {
        if w == 0 || h == 0 || w > 10_000_000 || h > 10_000_000 {
            return None;
        }
        let pixels = u64::from(w).checked_mul(u64::from(h))?;
        if pixels > 64_000_000 {
            return None;
        }
        usize::try_from(pixels).ok()?.checked_mul(4)
    }

    /// The flattened bitmap, colour-managed to sRGB for native clipboard interoperability.
    pub fn bitmap(&self) -> Result<(u32, u32, Vec<u8>)> {
        check_area(self.bounds)?;
        let (w, h) = (self.bounds.width(), self.bounds.height());
        let len = Self::bitmap_len(w, h).ok_or_else(|| EngineError::Other("Clipboard bitmap has an invalid size".into()))?;
        let from = self.profile.clone().unwrap_or_else(|| crate::color_cmds::working_profile(self.surface.format().mode));
        let to = photocraft_cms::Builtin::Srgb.profile();
        let surface = if from.content_hash() == to.content_hash() && self.surface.format().mode == photocraft_color::ColorMode::Rgb {
            self.surface.clone()
        } else {
            let t = photocraft_cms::transform::cached(&from, to, Default::default())
                .map_err(|e| EngineError::Other(format!("Clipboard colour conversion: {e}")))?;
            crate::color_cmds::convert_surface(&self.surface, self.surface.format().mode, PixelFormat::RGBA8, &t)
        };
        let mut px = Vec::new();
        px.try_reserve_exact(len / 4).map_err(|_| EngineError::Other("Not enough memory for the clipboard bitmap".into()))?;
        px.resize(len / 4, [0u8; 4]);
        surface.read_rgba8_into(self.bounds, &mut px);
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(|_| EngineError::Other("Not enough memory for the clipboard bitmap".into()))?;
        bytes.extend(px.into_iter().flatten());
        Ok((w, h, bytes))
    }
}

mod layers;

#[cfg(test)]
mod clipboard_tests;

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Paste and New from Clipboard need only a clipboard: with no document open, they make one.
fn has_clip_only(s: &Session) -> std::result::Result<(), String> {
    s.clipboard.as_ref().map(|_| ()).ok_or_else(|| "the clipboard is empty".into())
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

fn has_clip(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    s.clipboard.as_ref().map(|_| ()).ok_or_else(|| "the clipboard is empty".into())
}

fn active_id(s: &Session) -> Result<LayerId> {
    s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into()))
}

/// The pixels of `src` inside the selection (or all of them), as a straight-alpha surface.
fn lift(src: &Surface, sel: Option<&Surface>, canvas: Rect) -> Result<Clip> {
    let area = match sel {
        Some(m) => m.content_bounds().intersect(&canvas),
        None => src.content_bounds().intersect(&canvas),
    };
    lift_area(src, sel, area)
}

/// A plane's default pixels are real opaque values, so its area is supplied explicitly.
fn lift_area(src: &Surface, sel: Option<&Surface>, area: Rect) -> Result<Clip> {
    let fmt = src.format();
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let mut out = Surface::new(with_alpha);
    if area.is_empty() {
        return Ok(Clip::pixels(out, Rect::EMPTY));
    }
    check_area(area)?;
    // Only the selected area is read: converting the whole layer (a 24 MP Background gaining
    // alpha) cost about a second per Layer via Copy (#668).
    let n = with_alpha.channels();
    let mut px = if fmt.alpha {
        src.read_region(area)
    } else {
        let opaque = src.read_region(area);
        let k = fmt.channels();
        let mut v = Vec::with_capacity(opaque.len() / k.max(1) * n);
        for p in opaque.chunks_exact(k.max(1)) {
            v.extend_from_slice(p);
            v.push(1.0);
        }
        v
    };
    if let Some(m) = sel {
        let mask = m.read_region(area);
        let mk = m.format().channels().max(1);
        for (p, a) in px.chunks_exact_mut(n).zip(mask.chunks_exact(mk)) {
            p[n - 1] *= a[0];
        }
    }
    out.write_region(area, &px);
    out.prune();
    let bounds = out.content_bounds();
    Ok(Clip::pixels(out, bounds))
}

/// Bound clipboard allocations and coordinate arithmetic before raster/compositor helpers.
fn check_area(r: Rect) -> Result<()> {
    let pixels = u64::from(r.width()).checked_mul(u64::from(r.height()));
    if [r.x0, r.y0, r.x1, r.y1].iter().any(|c| i64::from(*c).abs() > 10_000_000) || pixels.is_none_or(|n| n > 64_000_000) {
        return Err(EngineError::Other("Clipboard area is too large (limit: 64 million pixels, coordinates within ±10 million)".into()));
    }
    Ok(())
}

fn clipboard_params(p: &Value) -> Result<bool> {
    let bad = |msg: &str| EngineError::Other(format!("Invalid clipboard parameters: {msg}"));
    if !p.is_object() && !p.is_null() {
        return Err(bad("expected an object"));
    }
    if let Some(target) = p.get("target") {
        let valid = matches!(target.as_str(), Some("pixels" | "mask" | "quickMask"))
            || target.get("channel").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).is_some();
        if !valid {
            return Err(bad("target must be pixels, mask, quickMask or {channel: index}"));
        }
    }
    match p.get("pixels") {
        None => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| bad("pixels must be a boolean")),
    }
}

/// Merged composite of the visible document as a surface in the document's format.
fn merged_surface(s: &Session) -> Result<Surface> {
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let fmt = doc.pixel_format();
    let surface = photocraft_compose::flatten_to_surface(doc, PixelFormat::new(photocraft_color::ColorMode::Rgb, fmt.sample, true), None);
    // The compositor emits RGB in its composite profile. Encode that into the document
    // profile before tagging the pixel clipboard; generic RGBA-to-CMYK/Gray math is not ICC.
    let clip = Clip { surface: surface.clone(), bounds: doc.bounds(), layers: None, profile: Some(crate::color_cmds::composite_profile(doc)) };
    convert_clip_pixels(s, &clip, &surface, doc, PixelFormat::new(fmt.mode, fmt.sample, true))
}

fn copy(s: &mut Session, p: &Value, merged: bool) -> Result<Value> {
    let pixels = clipboard_params(p)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let target = crate::channel_cmds::target_of(p);
    let color = if p.get("target").is_none() {
        match d.channel_view.target {
            crate::channel_cmds::ChannelTarget::Color(k) => Some(k),
            _ => None,
        }
    } else {
        None
    };
    let mut clip = if merged {
        check_area(canvas)?;
        lift(&merged_surface(s)?, d.doc.selection.as_ref(), canvas)?
    } else if target != crate::channel_cmds::Target::Pixels || color.is_some() {
        let plane = copy_plane(s, p, color)?;
        // Copy the whole plane / selection, including tile-free black or white defaults.
        let area = d.doc.selection.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
        check_area(area)?;
        lift_area(&plane, d.doc.selection.as_ref(), area)?
    } else if !pixels && d.doc.selection.is_none() {
        layers::capture(s)?
    } else {
        let id = d.active_layer.ok_or(EngineError::Other("no active layer".into()))?;
        let surf = d.doc.layer(id).and_then(Layer::surface).ok_or(EngineError::Other("the active layer has no pixels".into()))?;
        lift(surf, d.doc.selection.as_ref(), canvas)?
    };
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let b = clip.bounds;
    if clip.profile.is_none() {
        clip.profile = Some(if merged || (target == crate::channel_cmds::Target::Pixels && color.is_none()) {
            crate::color_cmds::document_profile(&d.doc)
        } else {
            crate::color_cmds::working_profile(clip.surface.format().mode)
        });
    }
    s.clipboard = Some(clip);
    Ok(json!({"bounds": [b.x0, b.y0, b.width(), b.height()]}))
}

fn copy_plane(s: &Session, p: &Value, color: Option<usize>) -> Result<Surface> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &st.doc;
    let id = st.active_layer;
    let source = match crate::channel_cmds::target_of(p) {
        crate::channel_cmds::Target::Mask => {
            &doc.layer(id.ok_or_else(|| EngineError::Other("no active layer".into()))?)
                .and_then(|l| l.mask.as_ref())
                .ok_or_else(|| EngineError::Other("the active layer has no mask".into()))?
                .surface
        }
        crate::channel_cmds::Target::Alpha(i) => &doc.channels.get(i).ok_or_else(|| EngineError::Other(format!("no alpha channel {i}")))?.surface,
        crate::channel_cmds::Target::QuickMask => &doc.quick_mask.as_ref().ok_or_else(|| EngineError::Other("not in Quick Mask mode".into()))?.surface,
        crate::channel_cmds::Target::Pixels => doc
            .layer(id.ok_or_else(|| EngineError::Other("no active layer".into()))?)
            .and_then(Layer::surface)
            .ok_or_else(|| EngineError::Other("the active layer has no pixels".into()))?,
    };
    let Some(k) = color else { return Ok(source.clone()) };
    if k >= source.format().mode.color_channels() {
        return Err(EngineError::Other("no such colour channel".into()));
    }
    let fmt = PixelFormat::new(photocraft_color::ColorMode::Grayscale, doc.depth, false);
    let mut out = Surface::new(fmt);
    let area = doc.selection.as_ref().map_or(doc.bounds(), |m| m.content_bounds().intersect(&doc.bounds()));
    check_area(area)?;
    let values: Vec<f32> = source.read_region(area).chunks_exact(source.channels()).filter_map(|px| px.get(k).copied()).collect();
    out.write_region(area, &values);
    Ok(out)
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
    let bg = crate::extra_cmds::is_background(doc.layer(id).ok_or(EngineError::NoLayer(id))?);
    let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
    if bg {
        crate::pixels::fill_surface(surf, area, background, sel, true);
    } else {
        crate::pixels::clear_surface(surf, area, sel);
    }
    surf.prune();
    Ok(())
}

/// Paste as a new layer (or into the targeted mask or channel: [`paste_to_target`]). `in_place` keeps the original position; otherwise the pixels are centred
/// on `center` (the view centre from the UI) or the canvas, unless they already overlap the canvas.
fn paste(s: &mut Session, p: &Value, in_place: bool) -> Result<Value> {
    let pixels = clipboard_params(p)?;
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    check_area(clip.bounds)?;
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("The clipboard image has no size; copy again before pasting".into()));
    }
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("the clipboard image has no size".into()));
    }
    // Validate even with no document open, so malformed placement does not create a document.
    let _ = paste_offset(&clip, clip.bounds, p, in_place)?;
    let Some(d) = s.active() else {
        // Nothing open to paste into: the clipboard becomes a document of its own (#368).
        return new_from_clipboard(s, pixels);
    };
    if crate::channel_cmds::target_of(p) != crate::channel_cmds::Target::Pixels {
        return paste_to_target(s, p, in_place, None, "Paste");
    }
    let canvas = d.doc.bounds();
    let fmt = d.doc.pixel_format();
    let (dx, dy) = paste_offset(&clip, canvas, p, in_place)?;
    if !pixels && let Some(source) = &clip.layers {
        return layers::paste(s, source, dx, dy);
    }
    let moved = shifted(&clip.surface, dx, dy);
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let surf = convert_clip_pixels(s, &clip, &moved, &d.doc, target)?;
    let id = s.edit("Paste", |doc, active| {
        let mut l = Layer::raster(doc.next_layer_name("Layer"), target);
        *crate::pixels_mut(&mut l)? = surf;
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        doc.selection = None;
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "offset": [dx, dy]}))
}

/// How far a paste moves the clipboard pixels: not at all in place (or when they lie on the
/// canvas and no `center` is given), else onto `center` (the view centre from the UI) or the
/// canvas centre.
fn paste_offset(clip: &Clip, canvas: Rect, p: &Value, in_place: bool) -> Result<(i32, i32)> {
    let c = match p.get("center") {
        None => None,
        Some(v) => {
            let xy = v.as_array().filter(|a| a.len() == 2).and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
            match xy {
                Some((x, y)) if x.is_finite() && y.is_finite() && x.abs() <= 1e7 && y.abs() <= 1e7 => Some((x, y)),
                _ => return Err(EngineError::Other("Invalid clipboard center: expected finite [x, y] within ±10 million pixels".into())),
            }
        }
    };
    if in_place || (clip.bounds.intersect(&canvas) == clip.bounds && p.get("center").is_none()) {
        return Ok((0, 0));
    }
    let (cx, cy) = c.unwrap_or(((f64::from(canvas.x0) + f64::from(canvas.x1)) / 2.0, (f64::from(canvas.y0) + f64::from(canvas.y1)) / 2.0));
    let b = clip.bounds;
    Ok(((cx - (f64::from(b.x0) + f64::from(b.x1)) / 2.0).round() as i32, (cy - (f64::from(b.y0) + f64::from(b.y1)) / 2.0).round() as i32))
}

fn convert_clip_pixels(s: &Session, clip: &Clip, surface: &Surface, doc: &Document, target: PixelFormat) -> Result<Surface> {
    let from = clip.profile.clone().unwrap_or_else(|| crate::color_cmds::working_profile(surface.format().mode));
    let to = crate::color_cmds::document_profile(doc);
    let out = if from.content_hash() == to.content_hash() && surface.format().mode == target.mode {
        surface.clone()
    } else {
        let transform = photocraft_cms::transform::cached(
            &from,
            &to,
            photocraft_cms::TransformOptions { intent: s.color.settings.intent(), bpc: s.color.settings.bpc, ..Default::default() },
        )
        .map_err(|e| EngineError::Other(format!("Clipboard colour conversion: {e}")))?;
        crate::color_cmds::convert_surface(surface, surface.format().mode, target, &transform)
    };
    Ok(if out.format() == target { out } else { out.convert(target) })
}

fn shifted(surface: &Surface, dx: i32, dy: i32) -> Surface {
    if dx == 0 && dy == 0 { surface.clone() } else { photocraft_algo::resample::translate_surface(surface, dx, dy) }
}

/// Paste into the targeted layer mask, alpha channel or Quick Mask (`"target"`) instead of as a
/// new layer (Photoshop, #1035): the pasted pixels' luminosity, placed as a paste places them,
/// over what was there. Their transparency, and `limit` (Paste Into / Outside), let it show
/// through. One history step.
pub(crate) fn paste_to_target(s: &mut Session, p: &Value, in_place: bool, limit: Option<&Surface>, label: &str) -> Result<Value> {
    clipboard_params(p)?;
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    check_area(clip.bounds)?;
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("The clipboard image has no size; copy again before pasting".into()));
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let layer = d.active_layer;
    let (dx, dy) = paste_offset(&clip, canvas, p, in_place)?;
    let src = shifted(&clip.surface, dx, dy);
    let area = src.content_bounds().intersect(&canvas);
    s.edit(label, |doc, _| {
        let (surf, _) = crate::channel_cmds::target_surface(doc, layer, p)?;
        crate::fill_cmds::composite_over(surf, &src, area, limit);
        surf.prune();
        doc.selection = None;
        Ok(())
    })?;
    Ok(json!({"target": p.get("target"), "offset": [dx, dy]}))
}

/// A new document the size of the clipboard extent, retaining its copied layers or bitmap
/// in the source format (#368).
fn new_from_clipboard(s: &mut Session, pixels: bool) -> Result<Value> {
    let mut clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    if pixels {
        clip.layers = None;
    }
    let b = clip.bounds;
    check_area(b)?;
    let (w, h) = (b.width(), b.height());
    if w == 0 || h == 0 {
        return Err(EngineError::Other("the clipboard image has no size".into()));
    }
    if let Some(source) = &clip.layers {
        let mut dest = Document::new("Untitled", photocraft_geom::Size::new(w, h), source.mode, source.depth);
        dest.icc_profile = source.icc_profile.clone();
        let mut doc = layers::prepare(s, source, &dest, -b.x0, -b.y0)?;
        doc.id = dest.id;
        doc.name = "Untitled".into();
        doc.size = photocraft_geom::Size::new(w, h);
        let i = s.add_document(doc, None);
        return Ok(json!({"document": i, "width": w, "height": h}));
    }
    let fmt = clip.surface.format();
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let moved = if b.x0 == 0 && b.y0 == 0 { clip.surface } else { photocraft_algo::resample::translate_surface(&clip.surface, -b.x0, -b.y0) };
    let mut doc = Document::new("Untitled", photocraft_geom::Size::new(w, h), fmt.mode, fmt.sample);
    doc.icc_profile = clip.profile.map(|p| p.to_bytes());
    let mut l = Layer::raster(doc.next_layer_name("Layer"), target);
    *crate::pixels_mut(&mut l)? = if moved.format() == target { moved } else { moved.convert(target) };
    doc.layers.push(l);
    let i = s.add_document(doc, None);
    Ok(json!({"document": i, "width": w, "height": h}))
}

fn layer_via(s: &mut Session, cut: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = active_id(s)?;
    let has_sel = d.doc.selection.is_some();
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
        let nid = doc.insert_above(Some(id), l);
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
        let is_background = base.name == "Background" && base.locks.transparency;
        // Composite of visible layers only (hidden ones stay where they are).
        let mut solo = doc.clone();
        solo.layers.retain(|l| l.visible);
        let buf = photocraft_compose::flatten(&solo);
        let buf = if is_background { buf.over_background([1.0, 1.0, 1.0]) } else { buf };
        let fmt = doc.pixel_format();
        let fmt = if is_background { fmt } else { PixelFormat::new(fmt.mode, fmt.sample, true) };
        let data: Vec<f32> = buf.px.iter().flat_map(|p| photocraft_raster::from_rgba(&fmt, *p)).collect();
        let mut merged = Layer::raster(base.name.clone(), fmt);
        merged.locks = base.locks;
        let surf = crate::pixels_mut(&mut merged)?;
        surf.write_region(doc.bounds(), &data);
        surf.prune();
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
        crate::pixels::adjust_surface(surf, &adj, sel.as_ref(), mode);
        Ok(())
    })?;
    Ok(Value::Null)
}

fn toggle_last_state(s: &mut Session) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let redo = st.history.can_redo();
    Ok(json!(if redo { s.redo() } else { s.undo() }))
}

/// Edit › Transform › Again: replay the last `edit.transform` on the active layer.
fn transform_again(s: &mut Session) -> Result<Value> {
    let p = transform_again_params(s)?;
    s.execute("edit.transform", p)
}

/// Transform Again on a copy (⌥⇧⌘T, no menu item): duplicates the active layer and repeats the
/// last transform on the copy, as one history step, so pressing it again steps and repeats
/// (#352). The transform is worked out before anything changes, so a refusal leaves no copy.
fn transform_again_copy(s: &mut Session) -> Result<Value> {
    let p = transform_again_params(s)?;
    let id = active_id(s)?;
    s.execute("layer.duplicate", json!({"layer": id.0}))?;
    match s.execute("edit.transform", p) {
        Ok(r) => {
            let st = s.active_mut().ok_or(EngineError::NoDocument)?;
            st.history.purge_last();
            st.history.set_current_label("Transform Again");
            Ok(r)
        }
        Err(e) => {
            // The copy (e.g. of a layer whose position is locked) couldn't be transformed: take
            // it back, leaving nothing to redo.
            s.undo();
            if let Some(st) = s.active_mut() {
                st.history.clear_redo();
            }
            Err(e)
        }
    }
}

/// The `edit.transform` params that repeat the last transform on the active layer.
fn transform_again_params(s: &Session) -> Result<Value> {
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
            let l = d.doc.layer(active_id(s)?).ok_or(EngineError::Other("no active layer".into()))?;
            let b = crate::transform_cmds::transform_bounds(&d.doc, l);
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

/// View › New Guide / guide moves (undoable, like Photoshop's "New Guide"/"Move Guide" states).
fn guide_cmd(s: &mut Session, p: &Value, op: &str) -> Result<Value> {
    let vertical = p.get("orientation").and_then(Value::as_str).unwrap_or("horizontal") == "vertical";
    let pos = p.get("position").and_then(Value::as_f64).map(|v| v as f32);
    let index = p.get("index").and_then(Value::as_u64).map(|v| v as usize);
    let label = match op {
        "new" => "New Guide",
        "move" => "Move Guide",
        "delete" => "Delete Guide",
        _ => "Clear Guides",
    };
    s.edit(label, |doc, _| {
        let g = if vertical { &mut doc.guides.vertical } else { &mut doc.guides.horizontal };
        match op {
            "new" => g.push(pos.ok_or(EngineError::Other("missing `position`".into()))?),
            "move" => {
                let i = index.filter(|i| *i < g.len()).ok_or(EngineError::Other("no such guide".into()))?;
                g[i] = pos.ok_or(EngineError::Other("missing `position`".into()))?;
            }
            "delete" => {
                let i = index.filter(|i| *i < g.len()).ok_or(EngineError::Other("no such guide".into()))?;
                g.remove(i);
            }
            _ => {
                doc.guides.horizontal.clear();
                doc.guides.vertical.clear();
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!("edit.cut", "Cut", &["Edit"], Some("Cmd+X"), "{}", has_pixels, |s, _| {
            // Refuse a locked layer before copying, so a refused Cut leaves the clipboard alone.
            let id = active_id(s)?;
            crate::commands::check_pixels_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
            let r = copy(s, &json!({"pixels": true}), false)?;
            let bg = s.tools.background;
            s.edit("Cut Pixels", |doc, _| clear_selected(doc, id, bg))?;
            Ok(r)
        }),
        spec!(
            "edit.copy",
            "Copy",
            &["Edit"],
            Some("Cmd+C"),
            concat!(
                r#"{"pixels":bool=false (force pixel copy; otherwise without a selection copies selected layers and their masks/styles),"#,
                r#""target":"pixels"|"mask"|"quickMask"|{"channel":i}=source plane (defaults to the active mask/channel)"#,
                "}"
            ),
            has_doc,
            |s, p| copy(s, p, false)
        ),
        spec!("edit.copyMerged", "Copy Merged", &["Edit"], Some("Cmd+Shift+C"), "{} (flattened visible composite; no editable masks)", has_doc, |s, p| copy(
            s, p, true
        )),
        spec!(
            "edit.paste",
            "Paste",
            &["Edit"],
            Some("Cmd+V"),
            concat!(
                r#"{"center":[x,y]? (view centre; default keeps the position when it fits the canvas),"pixels":bool=false (force bitmap paste),"#,
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
            "{} (a new document the size of the clipboard extent, retaining copied layers or a bitmap)",
            has_clip_only,
            |s, _| new_from_clipboard(s, false)
        ),
        spec!(
            "edit.pasteSpecial.pasteInPlace",
            "Paste in Place",
            &["Edit", "Paste Special"],
            Some("Cmd+Shift+V"),
            concat!(r#"{"pixels":bool=false (force bitmap paste),"#, paste_target!(), "}"),
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
        spec!("view.newGuide", "New Guide…", &["View"], None, r##"{"orientation":"horizontal|vertical","position":px}"##, has_doc, |s, p| guide_cmd(
            s, p, "new"
        )),
        spec!("view.moveGuide", "Move Guide", &[], None, r##"{"orientation":"horizontal|vertical","index":n,"position":px}"##, has_doc, |s, p| guide_cmd(
            s, p, "move"
        )),
        spec!("view.deleteGuide", "Delete Guide", &[], None, r##"{"orientation":"horizontal|vertical","index":n}"##, has_doc, |s, p| guide_cmd(s, p, "delete")),
        spec!("view.clearGuides", "Clear Guides", &["View"], None, "{}", has_doc, |s, p| guide_cmd(s, p, "clear")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 50, 30), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s
    }

    fn active_bounds(s: &Session) -> Rect {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
    }

    /// #368: the clipboard as a document of its own, from New from Clipboard or a Paste with
    /// nothing open, at every depth.
    #[test]
    fn new_from_clipboard_and_paste_with_no_document() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            assert!(!s.is_enabled("file.newFromClipboard") && !s.is_enabled("edit.paste"), "nothing copied yet");
            assert!(s.execute("file.newFromClipboard", json!({})).is_err());
            s.execute("file.new", json!({"width": 100, "height": 100, "depth": depth})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
            s.execute("select.rect", json!({"x": 10, "y": 20, "width": 30, "height": 15})).unwrap();
            s.execute("edit.copy", json!({})).unwrap();
            // With a document open: a second document, sized to the copy, its pixels at the origin.
            let r = s.execute("file.newFromClipboard", json!({})).unwrap();
            assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(30), Some(15)), "{r}");
            assert_eq!(s.documents().len(), 2);
            let d = &s.active().unwrap().doc;
            assert_eq!((d.size.width, d.size.height, d.layers.len()), (30, 15, 1));
            assert_eq!(d.pixel_format().sample, s.documents()[0].doc.pixel_format().sample, "{depth}-bit kept");
            assert_eq!(active_bounds(&s), Rect::new(0, 0, 30, 15));
            let px = d.layers[0].surface().unwrap().rgba(0, 0);
            assert!(px[0] > 0.99 && px[1] < 0.01 && px[3] > 0.99, "{px:?}");
            // With nothing open, Paste makes the document too.
            while s.active().is_some() {
                s.execute("file.close", json!({})).unwrap();
            }
            assert!(s.is_enabled("edit.paste") && !s.is_enabled("edit.pasteSpecial.pasteInPlace"));
            s.execute("edit.paste", json!({})).unwrap();
            assert_eq!(s.documents().len(), 1);
            assert_eq!(active_bounds(&s), Rect::new(0, 0, 30, 15));
            assert!(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).is_ok(), "into the new document");
        }
    }

    #[test]
    fn copy_paste_in_place_and_centred() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 30, 30));
        assert!(s.active().unwrap().doc.selection.is_none());
        s.execute("edit.paste", json!({"center": [70, 70]})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(60, 60, 80, 80));
        assert_eq!(s.active().unwrap().doc.layers.len(), 4);
    }

    #[test]
    fn paste_with_malformed_center_does_not_panic() {
        // Invalid placement is actionable and leaves the document unchanged (Rule 9).
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        for center in [json!([5]), json!([]), json!("nope"), json!([1, 2, 3])] {
            assert!(s.execute("edit.paste", json!({ "center": center })).is_err(), "center={center}");
        }
    }

    #[test]
    fn copy_paste_between_documents() {
        // Copy in one tab, paste into another: the clipboard is shared across documents.
        let mut s = session(); // doc A (100x100): red rect at (10,10)-(50,30)
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        // Open a second document (a new tab), which becomes active.
        s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        assert_eq!(s.active_index().unwrap(), 1, "the new document is the active tab");
        let before = s.active().unwrap().doc.layers.len();
        s.execute("edit.paste", json!({})).unwrap();
        assert_eq!(s.active_index().unwrap(), 1, "paste stays in the second tab");
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers.len(), before + 1, "pasted layer added to the second document");
        let pasted = d.layers.last().unwrap().surface().unwrap();
        let cb = pasted.content_bounds();
        let (cx, cy) = (cb.x0 + cb.width() as i32 / 2, cb.y0 + cb.height() as i32 / 2);
        let mut px = [[0.0f32; 4]; 1];
        pasted.read_rgba_into(Rect::new(cx, cy, cx + 1, cy + 1), &mut px);
        assert!(px[0][0] > 0.9 && px[0][1] < 0.1 && px[0][2] < 0.1, "pasted content is red: {:?}", px[0]);
    }

    /// #1035: a `depth`-bit document with a blue-grey square copied from layer `src` (at
    /// (5,5)-(15,15)) and an active layer `m` above it with a hide-all mask.
    fn mask_target_session(depth: u32) -> (Session, LayerId, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
        let src = LayerId(s.execute("layer.new.layer", json!({"name": "src"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let m = LayerId(s.execute("layer.new.layer", json!({"name": "m"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        (s, src, m)
    }

    /// The luminosity of `src`'s pixel at (`x`, `y`): what a paste into a mask writes there.
    fn luminosity(s: &Session, src: LayerId, x: i32, y: i32) -> f32 {
        let p = s.active().unwrap().doc.layer(src).unwrap().surface().unwrap().rgba(x, y);
        photocraft_color::convert::rgb_to_gray([p[0], p[1], p[2]])
    }

    fn mask_at(s: &Session, id: LayerId, x: i32, y: i32) -> f32 {
        s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().unwrap().value(x, y)
    }

    #[test]
    fn paste_into_a_targeted_mask_writes_the_luminosity_in_one_step() {
        for depth in [8, 16, 32] {
            let (mut s, src, m) = mask_target_session(depth);
            let lum = luminosity(&s, src, 10, 10);
            assert!(lum > 0.05 && lum < 0.95, "{depth}-bit: a mid grey ({lum})");
            let (layers, steps) = (s.active().unwrap().doc.layer_count(), s.active().unwrap().history.past_len());
            s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": "mask"})).unwrap();
            let st = s.active().unwrap();
            assert_eq!(st.doc.layer_count(), layers, "{depth}-bit: no new layer");
            assert_eq!(st.history.past_len(), steps + 1, "{depth}-bit: one history step");
            assert_eq!(st.active_layer, Some(m));
            assert!((mask_at(&s, m, 10, 10) - lum).abs() < 2.0 / 255.0, "{depth}-bit: {} vs {lum}", mask_at(&s, m, 10, 10));
            assert_eq!(mask_at(&s, m, 20, 20), 0.0, "{depth}-bit: the mask outside the paste is unchanged");
            assert!(st.doc.layer(m).unwrap().surface().unwrap().content_bounds().is_empty(), "{depth}-bit: the pixels are untouched");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(mask_at(&s, m, 10, 10), 0.0, "{depth}-bit: undo restores the mask");
        }
    }

    #[test]
    fn paste_follows_the_mask_view_and_is_placed_like_a_paste() {
        let (mut s, src, m) = mask_target_session(8);
        let lum = luminosity(&s, src, 10, 10);
        // ⌥-click the mask thumbnail: the canvas shows the mask, and pasting targets it.
        s.execute(crate::mask_view_cmds::ID, json!({"layer": m.0, "mode": "gray"})).unwrap();
        let r = s.execute("edit.paste", json!({"center": [30, 30]})).unwrap();
        assert_eq!(r["offset"], json!([20, 20]));
        assert!((mask_at(&s, m, 30, 30) - lum).abs() < 2.0 / 255.0);
        assert_eq!(mask_at(&s, m, 10, 10), 0.0);
        assert_eq!(s.active().unwrap().doc.layers.len(), 3, "Background, src, m");
    }

    #[test]
    fn paste_into_and_outside_a_targeted_mask_keep_to_the_selection() {
        let (mut s, src, m) = mask_target_session(16);
        let lum = luminosity(&s, src, 10, 10);
        // The paste is centred on the selection, so lands in place: only the selection takes it.
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
        s.execute("edit.pasteSpecial.pasteInto", json!({"target": "mask"})).unwrap();
        assert!((mask_at(&s, m, 9, 9) - lum).abs() < 2.0 / 255.0);
        assert_eq!(mask_at(&s, m, 7, 7), 0.0, "pasted, but outside the selection");
        assert!(s.active().unwrap().doc.selection.is_none());
        assert_eq!(s.active().unwrap().doc.layers.len(), 3, "no new layer");
        s.execute("layer.layerMask.hideAll", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 4, "height": 4})).unwrap();
        s.execute("edit.pasteSpecial.pasteOutside", json!({"target": "mask"})).unwrap();
        assert_eq!(mask_at(&s, m, 9, 9), 0.0, "Paste Outside leaves the selection alone");
        assert!((mask_at(&s, m, 7, 7) - lum).abs() < 2.0 / 255.0);
    }

    #[test]
    fn paste_follows_a_targeted_alpha_channel() {
        let (mut s, src, _) = mask_target_session(8);
        let lum = luminosity(&s, src, 10, 10);
        s.execute("channel.new", json!({})).unwrap();
        s.execute("edit.pasteSpecial.pasteInPlace", json!({"target": {"channel": 0}})).unwrap();
        let c = &s.active().unwrap().doc.channels[0].surface;
        assert!((c.sample_channel(10, 10, 0) - lum).abs() < 2.0 / 255.0);
        assert_eq!(s.active().unwrap().doc.layers.len(), 3, "no new layer");
    }

    #[test]
    fn paste_into_a_missing_mask_or_channel_is_an_error() {
        let (mut s, _, m) = mask_target_session(8);
        s.edit("drop mask", |doc, _| {
            doc.layer_mut(m).unwrap().mask = None;
            Ok(())
        })
        .unwrap();
        let layers = s.active().unwrap().doc.layer_count();
        for target in [json!("mask"), json!({"channel": 7}), json!("quickMask")] {
            for id in ["edit.paste", "edit.pasteSpecial.pasteInPlace"] {
                assert!(s.execute(id, json!({ "target": target })).is_err(), "{id} {target}");
            }
        }
        assert_eq!(s.active().unwrap().doc.layer_count(), layers, "nothing pasted as a layer");
    }

    #[test]
    fn paste_between_different_color_modes() {
        // Copy from an RGB document, paste into a Grayscale one: the clipboard converts to the
        // target document's format (a realistic between-tabs case that must not panic).
        let mut s = session(); // RGB, red rect
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("file.new", json!({"width": 120, "height": 120, "mode": "gray"})).unwrap();
        s.execute("edit.paste", json!({})).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.mode, photocraft_doc::ColorMode::Grayscale);
        let pasted = d.layers.last().unwrap().surface().unwrap();
        assert_eq!(pasted.format().mode, photocraft_color::ColorMode::Grayscale, "pasted layer is grayscale");
        assert!(!pasted.content_bounds().is_empty(), "pasted content exists");
    }

    #[test]
    fn transform_scale_grows_content() {
        let mut s = session(); // red rect (10,10)-(50,30): 40x20
        s.execute("edit.transform", json!({"matrix": [2, 0, 0, 2, 0, 0]})).unwrap();
        let d = &s.active().unwrap().doc;
        let cb = d.layers.last().unwrap().surface().unwrap().content_bounds();
        // Scaled ~2x about the origin (≈ 80x40 at ≈ (20,20)); allow ~1px bilinear edge bleed.
        assert!((cb.x0 - 20).abs() <= 2 && (cb.y0 - 20).abs() <= 2, "origin ~ (20,20): {cb:?}");
        assert!((cb.width() as i32 - 80).abs() <= 3 && (cb.height() as i32 - 40).abs() <= 3, "size ~ 80x40: {cb:?}");
    }

    #[test]
    fn cut_and_clear_on_the_background_fill_with_the_background_colour() {
        // The Background keeps its transparency lock: cutting fills with the background colour.
        let fresh = || {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
            s.tools.background = [1.0, 0.0, 0.0, 1.0];
            s.execute("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
            s
        };
        let px = |s: &Session, i: usize, x: i32, y: i32| s.active().unwrap().doc.layers[i].surface().unwrap().pixel(x, y);
        for cmd in ["edit.cut", "edit.clear", "layer.new.layerViaCut"] {
            let mut s = fresh();
            s.execute(cmd, json!({})).unwrap();
            let d = &s.active().unwrap().doc;
            assert!(crate::extra_cmds::is_background(&d.layers[0]), "{cmd}: still the Background");
            assert_eq!(px(&s, 0, 8, 8), vec![1.0, 0.0, 0.0, 1.0], "{cmd}: cut area is the background colour");
            assert_eq!(px(&s, 0, 30, 20), vec![1.0, 1.0, 1.0, 1.0], "{cmd}: outside untouched");
        }
        // Layer via Cut still lifts the original pixels onto the new layer.
        let mut s = fresh();
        s.execute("layer.new.layerViaCut", json!({})).unwrap();
        assert_eq!(px(&s, 1, 8, 8), vec![1.0, 1.0, 1.0, 1.0]);
        // Once it's a normal layer, cutting makes transparency again.
        let mut s = fresh();
        s.execute("layer.new.layerFromBackground", json!({})).unwrap();
        s.execute("edit.clear", json!({})).unwrap();
        assert_eq!(px(&s, 0, 8, 8)[3], 0.0);
    }

    #[test]
    fn layer_via_copy_of_a_background_keeps_colour_and_partial_selection() {
        // #668: lift reads only the selected area now; the result must be what converting the
        // whole layer gave: the colour, with the selection's coverage as alpha.
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": "#336699"})).unwrap();
            s.edit("partial selection", |doc, _| {
                let mut m = photocraft_raster::Surface::new(PixelFormat::GRAY8);
                m.fill_rect(Rect::new(5, 5, 15, 15), &[1.0]);
                m.fill_rect(Rect::new(15, 5, 20, 15), &[0.5]);
                doc.selection = Some(m);
                Ok(())
            })
            .unwrap();
            let id = s.execute("layer.new.layerViaCopy", json!({})).unwrap()["layer"].as_u64().unwrap();
            let d = s.active().unwrap();
            let surf = d.doc.layer(LayerId(id)).unwrap().surface().unwrap();
            assert!(surf.format().alpha, "@{depth}");
            let full = surf.read_region(Rect::new(6, 6, 7, 7));
            let half = surf.read_region(Rect::new(16, 6, 17, 7));
            for (got, want) in full.iter().zip([0.2, 0.4, 0.6, 1.0]) {
                assert!((got - want).abs() < 0.01, "@{depth}: {full:?}");
            }
            assert!((half[3] - 0.5).abs() < 0.01 && (half[0] - 0.2).abs() < 0.01, "@{depth}: {half:?}");
            assert_eq!(surf.content_bounds(), Rect::new(5, 5, 20, 15), "@{depth}");
        }
    }

    #[test]
    fn cut_clears_and_layer_via_copy_cut() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 20})).unwrap();
        s.execute("edit.cut", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(20, 10, 50, 30));
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 20})).unwrap();
        s.execute("layer.new.layerViaCut", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 20, 30));
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers[1].surface().unwrap().content_bounds(), Rect::new(20, 10, 50, 30));
        // Without a selection ⌘J duplicates the layer.
        let mut s = session();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers.len(), 3);
        assert!(d.layers[2].name.ends_with("copy"));
    }

    /// #1096: Layer via Copy and Cut deselect, as Photoshop does, so a following Free Transform
    /// moves the whole new layer; one undo brings back both the selection and the old layers.
    #[test]
    fn layer_via_copy_and_cut_deselect_in_one_step() {
        for cmd in ["layer.new.layerViaCopy", "layer.new.layerViaCut"] {
            let mut s = session();
            s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 20})).unwrap();
            let (layers, steps) = (s.active().unwrap().doc.layers.len(), s.active().unwrap().history.past_len());
            s.execute(cmd, json!({})).unwrap();
            let st = s.active().unwrap();
            assert!(st.doc.selection.is_none(), "{cmd}: deselected");
            assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers + 1, steps + 1), "{cmd}");
            assert!(s.undo());
            let st = s.active().unwrap();
            assert_eq!(st.doc.layers.len(), layers, "{cmd}: undo removes the layer");
            assert!(st.doc.selection.is_some(), "{cmd}: and restores the selection");
        }
    }

    fn active_content(s: &Session) -> LayerContent {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().content.clone()
    }

    /// Whole-layer Copy retains a smart object; a pixel selection copies its rendered pixels.
    /// Cut edits pixels, so it remains refused on smart objects.
    #[test]
    fn copy_a_smart_object_layer() {
        let mut s = session();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        assert!(s.is_enabled("edit.copy"));
        assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([10, 10, 40, 20]), "no selection: the whole layer");
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        assert!(matches!(active_content(&s), LayerContent::Smart(_)));
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 50, 30));
        assert_eq!(
            s.active().unwrap().doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().pixel(20, 20),
            vec![1.0, 0.0, 0.0, 1.0]
        );
        s.undo();
        assert!(matches!(active_content(&s), LayerContent::Smart(_)));
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([10, 10, 10, 10]), "only the selected part");
        let cut = s.execute("edit.cut", json!({})).unwrap_err().to_string();
        assert!(cut.contains("not directly editable"), "{cut}");
        assert!(matches!(active_content(&s), LayerContent::Smart(_)));
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        assert!(matches!(active_content(&s), LayerContent::Smart(_)), "⌘J without a selection duplicates the smart object");
        s.execute("layer.duplicate", json!({})).unwrap();
        assert!(matches!(active_content(&s), LayerContent::Smart(_)));
        assert_eq!(s.active().unwrap().doc.layers.len(), 4);
    }

    /// Type and group layers can be copied as layers without rasterization.
    #[test]
    fn copy_reads_any_layer_that_shows_pixels() {
        let mut s = session();
        s.execute("type.create", json!({"text": "Hi", "size": 30, "x": 20, "y": 60})).unwrap();
        assert!(!s.is_enabled("edit.cut"));
        assert!(s.execute("edit.copy", json!({})).is_ok());
        s.execute("layer.groupLayers", json!({})).unwrap();
        assert!(s.is_enabled("edit.copy"));
        assert!(s.execute("edit.copy", json!({})).is_ok());
    }

    #[test]
    fn merge_visible_keeps_hidden_layers() {
        let mut s = session();
        s.execute("layer.new.layer", json!({})).unwrap();
        let hidden = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setProps", json!({"layer": hidden.0, "visible": false})).unwrap();
        s.execute("layer.mergeVisible", json!({})).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.layers[0].name, "Background");
        assert_eq!(d.layers[1].id, hidden);
        assert_eq!(d.layers[0].surface().unwrap().pixel(20, 20)[..3], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn auto_tone_stretches_a_low_contrast_layer() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s.edit("grey ramp", |doc, _| {
            let surf = doc.layers[0].surface_mut().unwrap();
            for x in 0..64 {
                let v = 0.3 + 0.4 * x as f32 / 63.0;
                surf.fill_rect(Rect::new(x, 0, x + 1, 64), &[v, v * 0.9, v * 0.8, 1.0]);
            }
            Ok(())
        })
        .unwrap();
        for cmd in ["image.autoTone", "image.autoContrast", "image.autoColor"] {
            s.execute(cmd, json!({})).unwrap();
            let d = &s.active().unwrap().doc;
            let (a, b) = (d.layers[0].surface().unwrap().pixel(0, 0), d.layers[0].surface().unwrap().pixel(63, 0));
            // Auto Contrast keeps colour (shared points): only the extreme channels reach 0 and 1.
            let (lo, hi) = if cmd == "image.autoContrast" { (a[2], b[0]) } else { (a[0], b[0]) };
            assert!(lo < 0.05 && hi > 0.95, "{cmd}: {a:?} {b:?}");
            s.undo();
        }
    }

    /// #352: ⌥⇧⌘T steps and repeats: each press adds a copy one transform further on, and each
    /// is one undo step.
    #[test]
    fn transform_again_on_a_copy_steps_and_repeats() {
        let mut s = session();
        assert!(s.execute("edit.transform.againCopy", json!({})).is_err(), "nothing to repeat yet");
        assert_eq!(s.active().unwrap().doc.layers.len(), 2, "a refusal leaves no copy");
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 0]})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        for (i, x) in [30, 40].into_iter().enumerate() {
            s.execute("edit.transform.againCopy", json!({})).unwrap();
            let d = &s.active().unwrap().doc;
            assert_eq!(d.layers.len(), 3 + i);
            assert_eq!(active_bounds(&s), Rect::new(x, 10, x + 40, 30), "the copy moved one step further");
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1 + i, "one history step per press");
        }
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Transform Again"));
        // The original and the first copy stay where they were.
        let d = &s.active().unwrap().doc;
        let b = |i: usize| d.layers[i].surface().unwrap().content_bounds();
        assert_eq!((b(1), b(2)), (Rect::new(20, 10, 60, 30), Rect::new(30, 10, 70, 30)));
        // Undo removes the copy and its move together.
        s.undo();
        assert_eq!(s.active().unwrap().doc.layers.len(), 3);
        assert_eq!(active_bounds(&s), Rect::new(30, 10, 70, 30));
        // A copy that can't move (position locked) is taken back.
        let id = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setProps", json!({"layer": id.0, "locks": {"position": true}})).unwrap();
        let layers = s.active().unwrap().doc.layers.len();
        assert!(s.execute("edit.transform.againCopy", json!({})).unwrap_err().to_string().contains("locked"));
        assert_eq!(s.active().unwrap().doc.layers.len(), layers);
        assert!(!s.active().unwrap().history.can_redo(), "nothing to redo");
    }

    #[test]
    fn toggle_last_state_and_transform_again() {
        let mut s = session();
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 0]})).unwrap();
        s.execute("edit.transform.again", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(30, 10, 70, 30));
        s.execute("edit.toggleLastState", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(20, 10, 60, 30));
        s.execute("edit.toggleLastState", json!({})).unwrap();
        assert_eq!(active_bounds(&s), Rect::new(30, 10, 70, 30));
    }

    #[test]
    fn guides_are_undoable() {
        let mut s = session();
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 40})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 12.5})).unwrap();
        s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 60})).unwrap();
        let g = &s.active().unwrap().doc.guides;
        assert_eq!((g.vertical.clone(), g.horizontal.clone()), (vec![60.0], vec![12.5]));
        s.execute("view.deleteGuide", json!({"orientation": "horizontal", "index": 0})).unwrap();
        s.undo();
        assert_eq!(s.active().unwrap().doc.guides.horizontal, vec![12.5]);
        s.execute("view.clearGuides", json!({})).unwrap();
        assert!(s.active().unwrap().doc.guides.vertical.is_empty());
        assert!(s.execute("view.moveGuide", json!({"orientation": "vertical", "index": 3, "position": 1})).is_err());
    }
}
