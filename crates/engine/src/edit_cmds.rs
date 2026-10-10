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

/// Pixels on the session clipboard, with where they came from (for Paste in Place).
#[derive(Clone, Debug)]
pub struct Clip {
    pub surface: Surface,
    pub bounds: Rect,
    /// The copied layer itself, when Copy took a whole layer that Paste should recreate rather
    /// than rasterize (see [`whole_layer`]): a scratch document in the source's colour holding it.
    /// `surface` still holds its pixels, for Paste Into, masks, channels and other apps.
    pub layers: Option<Box<Document>>,
}

/// Photoshop's layer-based copy (CC 2018 on): with no selection, Copy takes the active layer
/// whole. Here that applies to smart objects, so Paste makes a smart object again instead of its
/// pixels (#2428); other layers paste as pixels.
fn whole_layer(doc: &Document, id: LayerId) -> Option<Box<Document>> {
    let l = doc.layer(id)?;
    if doc.selection.is_some() || !matches!(l.content, LayerContent::Smart(_)) || !crate::layer_copy_cmds::layered(doc.mode) {
        return None;
    }
    let mut copies = Document::new("", doc.size, doc.mode, doc.depth);
    copies.icc_profile = doc.icc_profile.clone();
    copies.resolution_dpi = doc.resolution_dpi;
    copies.patterns = doc.patterns.clone();
    copies.layers.push(l.clone());
    Some(Box::new(copies))
}

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

/// Copy only reads: any layer that shows pixels will do, so a smart object, type or shape layer
/// copies what it shows (Photoshop).
fn has_layer_pixels(s: &Session) -> std::result::Result<(), String> {
    active_layer(s)?.surface().map(|_| ()).ok_or_else(|| "the active layer has no pixels".into())
}

/// Copy also copies a targeted alpha channel, with or without a pixel layer (#2307).
fn can_copy(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_clip::copies_alpha_channel(s) { Ok(()) } else { has_layer_pixels(s) }
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
    let fmt = src.format();
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let area = match sel {
        Some(m) => m.content_bounds().intersect(&canvas),
        None => src.content_bounds().intersect(&canvas),
    };
    let mut out = Surface::new(with_alpha);
    if area.is_empty() {
        return Ok(Clip { surface: out, bounds: Rect::EMPTY, layers: None });
    }
    // Only the selected area is read: converting the whole layer (a 24 MP Background gaining
    // alpha) cost about a second per Layer via Copy (#668).
    let n = with_alpha.channels();
    let mut px = if fmt.alpha {
        crate::allocation::read_region(src, area, "copying pixels")?
    } else {
        let opaque = crate::allocation::read_region(src, area, "copying pixels")?;
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
        let mask = crate::allocation::read_region(m, area, "applying the selection to copied pixels")?;
        let mk = m.format().channels().max(1);
        for (p, a) in px.chunks_exact_mut(n).zip(mask.chunks_exact(mk)) {
            p[n - 1] *= a[0];
        }
    }
    out.try_write_region(area, &px).map_err(|e| EngineError::Other(format!("not enough memory while copying pixels ({e}); the document was not changed")))?;
    out.prune();
    let bounds = out.content_bounds();
    Ok(Clip { surface: out, bounds, layers: None })
}

/// Merged composite of the visible document as a surface in the document's format.
fn merged_surface(doc: &Document) -> Surface {
    let fmt = doc.pixel_format();
    photocraft_compose::flatten_to_surface(doc, PixelFormat::new(fmt.mode, fmt.sample, true), None)
}

fn copy(s: &mut Session, merged: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let clip = if merged {
        lift(&merged_surface(&d.doc), d.doc.selection.as_ref(), canvas)?
    } else {
        let id = d.active_layer.ok_or(EngineError::Other("no active layer".into()))?;
        let surf = d.doc.layer(id).and_then(Layer::surface).ok_or(EngineError::Other("the active layer has no pixels".into()))?;
        Clip { layers: whole_layer(&d.doc, id), ..lift(surf, d.doc.selection.as_ref(), canvas)? }
    };
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let b = clip.bounds;
    s.clipboard = Some(clip);
    Ok(json!({"bounds": [b.x0, b.y0, b.width(), b.height()]}))
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
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
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
    let (dx, dy) = paste_offset(&clip, canvas, p, in_place);
    if let Some(layers) = clip.layers.as_deref().filter(|_| crate::layer_copy_cmds::layered(d.doc.mode)) {
        return paste_layers(s, layers, dx, dy);
    }
    let moved = shifted(&clip.surface, dx, dy);
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let surf = if moved.format() == target { moved } else { moved.convert(target) };
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

/// Paste of a whole copied layer ([`whole_layer`]): fresh copies, converted to the document's
/// colour, moved as the pixels would be, above the active layer. A pasted smart object gets its
/// own contents, independent of the copied one (as across documents).
fn paste_layers(s: &mut Session, layers: &Document, dx: i32, dy: i32) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let mut copies = layers.clone();
    for l in &mut copies.layers {
        *l = l.duplicate();
    }
    crate::layer_copy_cmds::adapt_copies(s, &mut copies, &d.doc)?;
    crate::layer_copy_cmds::translate_copies(&mut copies, dx, dy);
    let patterns: Vec<_> = copies.patterns.iter().filter(|pat| !d.doc.patterns.iter().any(|q| q.id == pat.id)).cloned().collect();
    let id = s.edit("Paste", |doc, active| {
        doc.patterns.extend(patterns);
        for l in copies.layers {
            *active = Some(doc.insert_above(*active, l));
        }
        doc.selection = None;
        active.ok_or(EngineError::Other("nothing to paste".into()))
    })?;
    Ok(json!({"layer": id.0, "offset": [dx, dy]}))
}

/// How far a paste moves the clipboard pixels: not at all in place (or when they lie on the
/// canvas and no `center` is given), else onto `center` (the view centre from the UI) or the
/// canvas centre.
pub(crate) fn paste_offset(clip: &Clip, canvas: Rect, p: &Value, in_place: bool) -> (i32, i32) {
    if in_place || (clip.bounds.intersect(&canvas) == clip.bounds && p.get("center").is_none()) {
        return (0, 0);
    }
    let c = p.get("center").and_then(Value::as_array).filter(|a| a.len() >= 2).map(|a| (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)));
    let (cx, cy) = c.unwrap_or(((canvas.x0 + canvas.x1) as f64 / 2.0, (canvas.y0 + canvas.y1) as f64 / 2.0));
    let b = clip.bounds;
    ((cx - (b.x0 + b.x1) as f64 / 2.0).round() as i32, (cy - (b.y0 + b.y1) as f64 / 2.0).round() as i32)
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
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let layer = d.active_layer;
    let (dx, dy) = paste_offset(&clip, canvas, p, in_place);
    let src = shifted(&clip.surface, dx, dy);
    let area = src.content_bounds().intersect(&canvas);
    s.edit(label, |doc, _| {
        let (surf, _) = crate::channel_cmds::target_surface(doc, layer, p)?;
        crate::fill_cmds::composite_over(surf, &src, area, limit)?;
        surf.prune();
        doc.selection = None;
        Ok(())
    })?;
    Ok(json!({"target": p.get("target"), "offset": [dx, dy]}))
}

/// A new document the size of the clipboard image, holding it as its one layer, in the pixel
/// format it was copied in (#368).
fn new_from_clipboard(s: &mut Session) -> Result<Value> {
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    let b = clip.bounds;
    let (w, h) = (b.width(), b.height());
    if w == 0 || h == 0 {
        return Err(EngineError::Other("the clipboard image has no size".into()));
    }
    let fmt = clip.surface.format();
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let moved = if b.x0 == 0 && b.y0 == 0 { clip.surface } else { photocraft_algo::resample::translate_surface(&clip.surface, -b.x0, -b.y0) };
    let mut doc = Document::new("Untitled", photocraft_geom::Size::new(w, h), fmt.mode, fmt.sample);
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
        spec!("edit.cut", "Cut", &["Edit"], Some("Cmd+X"), "{} (a targeted colour channel cuts that channel only)", has_pixels, |s, p| {
            // Refuse a locked layer before copying, so a refused Cut leaves the clipboard alone.
            let id = active_id(s)?;
            crate::commands::check_pixels_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
            // A targeted colour channel cuts that channel only (#2698).
            if let Some(k) = crate::channel_clip::paste_color_target(s, p) {
                return crate::channel_clip::cut(s, id, k);
            }
            let r = copy(s, false)?;
            let bg = s.tools.background;
            s.edit("Cut Pixels", |doc, _| clear_selected(doc, id, bg))?;
            Ok(r)
        }),
        spec!("edit.copy", "Copy", &["Edit"], Some("Cmd+C"), "{} (a targeted colour or alpha channel copies that channel, as grayscale)", can_copy, |s, _| {
            match crate::channel_clip::copy(s)? {
                Some(r) => Ok(r),
                None => copy(s, false),
            }
        }),
        spec!("edit.copyMerged", "Copy Merged", &["Edit"], Some("Cmd+Shift+C"), "{}", has_doc, |s, _| copy(s, true)),
        spec!(
            "edit.paste",
            "Paste",
            &["Edit"],
            Some("Cmd+V"),
            concat!(
                r#"{"center":[x,y]? (view centre; default keeps the position when it overlaps the canvas),"#,
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

    #[test]
    fn injected_clear_buffer_failure_leaves_the_document_unchanged() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
        let revision = st.revision;

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.clear", json!({}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for erasing pixels")));
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
        assert_eq!(st.revision, revision);
    }

    #[test]
    fn injected_copy_buffer_failure_does_not_change_document_or_clipboard() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
        let revision = st.revision;

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.copy", json!({}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for copying pixels")));
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
        assert_eq!(st.revision, revision);
        assert!(s.clipboard.is_none());
    }

    #[test]
    fn injected_fill_result_reservation_failure_keeps_pixels_and_history() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
        let revision = st.revision;
        let history_len = st.history.past_len();

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.fill", json!({"color": "#00ff00"}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for blending pixels")));
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
        assert_eq!(st.revision, revision);
        assert_eq!(st.history.past_len(), history_len);
    }

    #[test]
    fn injected_paste_failure_does_not_change_document_or_clipboard() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 8, "height": 8})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        let before_clip = s.clipboard.clone();
        let st = s.active().unwrap();
        let before_doc = (*st.doc).clone();
        let revision = st.revision;
        let history_len = st.history.past_len();

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.paste", json!({}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for pasting pixels")));
        let st = s.active().unwrap();
        assert_eq!(*st.doc, before_doc);
        assert_eq!(st.revision, revision);
        assert_eq!(st.history.past_len(), history_len);
        let after_clip = s.clipboard.as_ref().expect("clipboard remains available");
        let before_clip = before_clip.as_ref().expect("clipboard was populated");
        assert_eq!(after_clip.bounds, before_clip.bounds);
        assert_eq!(after_clip.surface, before_clip.surface);
    }

    #[test]
    fn injected_transform_failure_keeps_pixels_and_history() {
        let mut s = session();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let before = st.doc.layer(id).unwrap().surface().unwrap().clone();
        let revision = st.revision;
        let history_len = st.history.past_len();

        crate::allocation::fail_next_for_test();
        let result = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 20, 30]}));

        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("not enough memory for transforming pixels")));
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface(), Some(&before));
        assert_eq!(st.revision, revision);
        assert_eq!(st.history.past_len(), history_len);
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
        // A short/empty `center` array must not panic (Rule 9) — it falls back to the canvas centre.
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        for center in [json!([5]), json!([]), json!("nope"), json!([1, 2, 3])] {
            // Fresh clipboard each time is unnecessary; copy persists. Must return Ok, never panic.
            assert!(s.execute("edit.paste", json!({ "center": center })).is_ok(), "center={center}");
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

    /// #2777: with several layers selected and no pixel selection, Layer via Copy (Cmd+J) copies
    /// every selected layer in one step, each copy above its original and the copies selected.
    /// With a pixel selection it still copies the active layer's selected pixels only.
    #[test]
    fn layer_via_copy_duplicates_every_selected_layer() {
        let mut s = session();
        let a = s.active().unwrap().active_layer.unwrap();
        let b = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.select", json!({"layer": a.0})).unwrap();
        s.execute("layer.select", json!({"layer": b.0, "mode": "toggle"})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        let st = s.active().unwrap();
        let ids: Vec<LayerId> = st.doc.layers.iter().map(|l| l.id).collect();
        assert_eq!(ids.len(), 5, "background, a, a copy, b, b copy");
        assert_eq!((ids[1], ids[3]), (a, b), "each copy sits above its original");
        let mut selected = st.selected_layers();
        selected.sort_by_key(|l| l.0);
        assert_eq!(selected, vec![ids[2], ids[4]], "the copies become the selection");
        assert_eq!(st.history.past_len(), steps + 1, "one undo step");
        assert!(s.undo());
        s.execute("layer.select", json!({"layer": b.0})).unwrap();
        s.execute("layer.select", json!({"layer": a.0, "mode": "toggle"})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 4, "a pixel selection copies one layer's pixels");
    }

    fn active_content(s: &Session) -> LayerContent {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().content.clone()
    }

    /// #541, #2428: with no selection, Copy takes a smart object whole and Paste makes a smart
    /// object again, with contents of its own; with a selection, the selected pixels paste as
    /// pixels. Cut edits pixels, so it's refused; Layer via Copy and Duplicate Layer copy the layer.
    #[test]
    fn copy_a_smart_object_layer() {
        let mut s = session();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let LayerContent::Smart(original) = active_content(&s) else { panic!("smart object") };
        assert!(s.is_enabled("edit.copy"));
        assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([10, 10, 40, 20]), "no selection: the whole layer");
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        let LayerContent::Smart(pasted) = active_content(&s) else { panic!("the paste is a smart object (#2428)") };
        assert_eq!(pasted.source, original.source, "the same contents");
        assert_ne!(pasted.contents_id, original.contents_id, "but not linked to the copied one");
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 50, 30));
        s.execute("edit.paste", json!({"center": [70, 70]})).unwrap();
        assert!(matches!(active_content(&s), LayerContent::Smart(_)));
        assert_eq!(active_bounds(&s), Rect::new(50, 60, 90, 80), "placed as pasted pixels are");
        s.undo();
        s.undo();
        assert!(matches!(active_content(&s), LayerContent::Smart(_)));
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        assert_eq!(s.execute("edit.copy", json!({})).unwrap()["bounds"], json!([10, 10, 10, 10]), "only the selected part");
        s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap();
        assert!(matches!(active_content(&s), LayerContent::Raster(_)), "a selection copies pixels");
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 20, 20));
        s.undo();
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

    /// #2428: a smart object copied in one document pastes into another as a smart object, in
    /// that document's bit depth.
    #[test]
    fn paste_a_smart_object_into_another_document() {
        let mut s = session();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        s.execute("file.new", json!({"width": 60, "height": 40, "depth": 16})).unwrap();
        s.execute("edit.paste", json!({})).unwrap();
        let LayerContent::Smart(sm) = active_content(&s) else { panic!("the paste is a smart object") };
        assert_eq!(sm.cache.map(|c| c.format().sample), Some(s.active().unwrap().doc.depth), "converted to 16 bits");
        assert_eq!(active_bounds(&s), Rect::new(10, 10, 50, 30), "on the canvas: in place");
    }

    /// Type and shape layers copy their rendered pixels too; groups and adjustments have none.
    #[test]
    fn copy_reads_any_layer_that_shows_pixels() {
        let mut s = session();
        s.execute("type.create", json!({"text": "Hi", "size": 30, "x": 20, "y": 60})).unwrap();
        assert!(!s.is_enabled("edit.cut"));
        assert!(s.execute("edit.copy", json!({})).is_ok());
        s.execute("layer.groupLayers", json!({})).unwrap();
        assert!(!s.is_enabled("edit.copy"));
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
