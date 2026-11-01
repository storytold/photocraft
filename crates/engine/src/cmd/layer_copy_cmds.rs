//! Copy layers into another open document (#589): what Photoshop does when layers are dragged from
//! the Layers panel, or with the Move tool from the canvas, onto another document's tab or window.
//! Photoshop has no menu item for it; `layer.copyToDocument` is the command the drag dispatches and
//! agents call.
//!
//! The copies keep their names, content, masks, styles and blending; they land above the
//! destination's active layer as one history step there, and the destination becomes the active
//! document. Pixels are converted to the destination's colour profile (Color Settings' intent and
//! black point compensation) and bit depth, as Photoshop converts dragged layers.

use std::collections::{HashMap, HashSet};

use photocraft_color::ColorMode;
use photocraft_doc::{Document, Layer, LayerContent, LayerId, SmartContentsId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "layer.copyToDocument";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.ok_or("no active layer")?;
    if s.documents().len() < 2 { Err("no other document is open".into()) } else { Ok(()) }
}

/// A document index param (`None` when absent), checked against the open documents.
fn doc_index(s: &Session, p: &Value, key: &str) -> Result<Option<usize>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(format!("`{key}` must be a document index")))?;
    if i >= s.documents().len() {
        return Err(bad(format!("no document {i} (there are {})", s.documents().len())));
    }
    Ok(Some(i))
}

/// An `[x, y]` param of finite numbers within ±10⁷ pixels.
fn point(p: &Value, key: &str) -> Result<Option<[f64; 2]>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let xy = v.as_array().filter(|a| a.len() == 2).and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?]));
    match xy {
        Some(xy) if xy.iter().all(|c| c.is_finite() && c.abs() <= 1e7) => Ok(Some(xy)),
        _ => Err(bad(format!("`{key}` must be [x, y] (pixels)"))),
    }
}

/// Modes whose layers convert through the colour engine.
pub(crate) fn layered(mode: ColorMode) -> bool {
    matches!(mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Cmyk | ColorMode::Lab)
}

fn center(r: Rect) -> [f64; 2] {
    [(f64::from(r.x0) + f64::from(r.x1)) / 2.0, (f64::from(r.y0) + f64::from(r.y1)) / 2.0]
}

/// The part of `l`'s pixels its layer masks show: what Photoshop centres a pasted layer on (#9681).
/// A layer with an enabled pixel or vector mask is cropped to the pixels where its effective mask
/// (as the compositor applies it: density, feather, the path's own coverage) and its ink are both
/// above zero; a group's ink is where its children show theirs. One below 100 % density hides
/// nothing completely and crops nothing; without a mask the layer's (or children's) bounds stand.
fn shown_bounds(l: &Layer) -> Option<Rect> {
    let masked = l.mask.as_ref().is_some_and(|m| m.enabled) || l.vector_mask.as_ref().is_some_and(|v| v.enabled);
    if !masked {
        return match &l.content {
            LayerContent::Group(g) => Some(g.children.iter().filter_map(shown_bounds).fold(Rect::EMPTY, |a, b| a.union(&b))).filter(|r| !r.is_empty()),
            _ => crate::layer_multi_cmds::layer_bounds(l),
        };
    }
    let shown = Shown::new(l)?;
    let b = shown.bounds;
    // Rows one at a time, so a large sparse layer never needs its whole rectangle in memory.
    let mut row = vec![false; b.width() as usize];
    let mut r = Rect::EMPTY;
    for y in b.y0..b.y1 {
        shown.row(y, b.x0, &mut row);
        if let (Some(a), Some(z)) = (row.iter().position(|h| *h), row.iter().rposition(|h| *h)) {
            let (a, z) = (b.x0.saturating_add(a as i32), b.x0.saturating_add(z as i32));
            r = r.union(&Rect::new(a, y, z.saturating_add(1), y.saturating_add(1)));
        }
    }
    (!r.is_empty()).then_some(r)
}

/// Where a layer shows ink, sampled a row at a time for [`shown_bounds`].
struct Shown<'a> {
    /// Where it can show anything: its pixels' bounds, or its children's.
    bounds: Rect,
    ink: Ink<'a>,
    mask: Option<ShownMask<'a>>,
}

enum Ink<'a> {
    /// Everywhere in the bounds: no alpha, or effects that reach past it.
    All,
    Alpha(&'a photocraft_raster::Surface),
    /// A group's: where its children show theirs.
    Children(Vec<Shown<'a>>),
}

enum ShownMask<'a> {
    /// An unfeathered pixel mask alone, read as it is (with its density).
    Pixel(&'a photocraft_doc::LayerMask),
    /// Pixel × vector mask, each feathered, as the compositor applies them.
    Combined(photocraft_raster::Surface),
}

impl<'a> Shown<'a> {
    fn new(l: &'a Layer) -> Option<Self> {
        let fx = photocraft_compose::effects::has_effects(l);
        let (bounds, ink) = match &l.content {
            LayerContent::Group(g) if !fx => {
                let kids: Vec<_> = g.children.iter().filter_map(Shown::new).collect();
                (kids.iter().fold(Rect::EMPTY, |a, k| a.union(&k.bounds)), Ink::Children(kids))
            }
            _ => (crate::layer_multi_cmds::layer_bounds(l)?, l.surface().filter(|s| s.format().alpha && !fx).map_or(Ink::All, Ink::Alpha)),
        };
        if bounds.is_empty() {
            return None;
        }
        // Everything that can reach into the bounds through either enabled mask's feather.
        let sigma = |f: f32| photocraft_compose::masks::feather_sigma(f);
        let pixel = l.mask.iter().filter(|m| m.enabled).map(|m| sigma(m.feather));
        let reach = pixel.chain(l.vector_mask.iter().filter(|v| v.enabled).map(|v| sigma(v.feather))).fold(0.0, f32::max);
        let combined = photocraft_compose::masks::combined_mask(l, photocraft_vector::feather::grow(bounds, reach));
        let mask = combined.map(ShownMask::Combined).or_else(|| l.mask.as_ref().filter(|m| m.enabled).map(ShownMask::Pixel));
        Some(Self { bounds, ink, mask })
    }

    /// Whether each pixel of row `y` from x `x0` (as many as `out` holds) shows ink, into `out`.
    fn row(&self, y: i32, x0: i32, out: &mut [bool]) {
        out.fill(false);
        let row = Rect::new(x0, y, x0.saturating_add(out.len() as i32), y.saturating_add(1)).intersect(&self.bounds);
        if row.is_empty() {
            return;
        }
        let Some(out) = out.get_mut((row.x0 - x0) as usize..(row.x1 - x0) as usize) else { return };
        let mut buf = Vec::new();
        match &self.ink {
            Ink::All => out.fill(true),
            Ink::Alpha(s) => {
                s.read_region_into(row, &mut buf);
                let n = s.channels();
                for (o, px) in out.iter_mut().zip(buf.chunks_exact(n)) {
                    *o = px[n - 1] > 0.0;
                }
            }
            Ink::Children(kids) => {
                let mut kid = vec![false; out.len()];
                for k in kids {
                    k.row(y, row.x0, &mut kid);
                    out.iter_mut().zip(&kid).for_each(|(o, h)| *o |= *h);
                }
            }
        }
        match &self.mask {
            Some(ShownMask::Pixel(m)) => m.values_into(row, &mut buf),
            Some(ShownMask::Combined(c)) => c.read_region_into(row, &mut buf),
            None => return,
        }
        out.iter_mut().zip(&buf).for_each(|(o, v)| *o &= *v > 0.0);
    }
}

/// Restore missing shape caches, or refresh every shape for a changed canvas, including groups.
fn prepare_shapes(layers: &mut [Layer], doc: &Document, new_canvas: bool) {
    for layer in layers {
        match &mut layer.content {
            LayerContent::Shape(shape) if new_canvas || shape.cache.is_none() => crate::vector_cmds::refresh_shape(doc, shape),
            LayerContent::Group(group) => prepare_shapes(&mut group.children, doc, new_canvas),
            _ => {}
        }
    }
}

/// Readies `copies` (a scratch document of the source's colour holding duplicated layers) for
/// landing in `dest`: smart objects get fresh contents ids, keeping only the sharing within the
/// batch (contents ids belong to a document, so the copies never join objects already in `dest`,
/// including another opening of the same file), and the pixels convert to `dest`'s profile and
/// bit depth.
pub(crate) fn adapt_copies(s: &Session, copies: &mut Document, dest: &Document) -> Result<()> {
    // Link-group IDs are scoped to a document. Two independently linked pairs can both
    // use group 1; copying one must not link it to the other's existing layers (#3045).
    // Remap each imported group once so links within the copied batch stay intact.
    let mut used: HashSet<u64> = dest.walk().into_iter().filter_map(|(_, _, l)| l.link_group).collect();
    let mut next_group = dest.unused_link_group().ok_or_else(|| EngineError::Other("no free linked-layer group id".into()))?;
    let groups: Vec<_> = copies.walk().into_iter().filter_map(|(_, _, l)| l.link_group.map(|group| (l.id, group))).collect();
    let mut remapped = HashMap::new();
    for (_, group) in &groups {
        if remapped.contains_key(group) {
            continue;
        }
        while used.contains(&next_group) {
            next_group = next_group.checked_add(1).unwrap_or(1);
        }
        remapped.insert(*group, next_group);
        used.insert(next_group);
        next_group = next_group.checked_add(1).unwrap_or(1);
    }
    for (id, group) in groups {
        if let Some(layer) = copies.layer_mut(id) {
            layer.link_group = remapped.get(&group).copied();
        }
    }
    let mut contents = HashMap::new();
    let smart_ids: Vec<_> = copies.walk().into_iter().filter_map(|(_, _, l)| matches!(l.content, LayerContent::Smart(_)).then_some(l.id)).collect();
    for id in smart_ids {
        if let Some(l) = copies.layer_mut(id)
            && let LayerContent::Smart(sm) = &mut l.content
        {
            sm.contents_id = *contents.entry(sm.contents_id).or_insert_with(SmartContentsId::fresh);
        }
    }
    if (copies.mode, &copies.icc_profile) != (dest.mode, &dest.icc_profile) {
        let profile = crate::color_cmds::document_profile(dest);
        crate::color_cmds::convert_document(copies, &profile, s.color.settings.intent(), s.color.settings.bpc)?;
    }
    if copies.depth != dest.depth {
        crate::image_cmds::for_each_surface(&mut copies.layers, true, &mut |surf, _| {
            let f = surf.format().with_sample(dest.depth);
            *surf = surf.convert(f);
        });
    }
    // Type keeps its pixels and takes the destination's point size, as in the reference
    // application (24 pt at 72 ppi becomes 5.76 pt at 300 ppi, with no transform). Left at the
    // source's, a copy re-laid out at the destination's resolution on its next edit (#9560).
    crate::canvas_geom::retarget_type_resolution(copies, dest.resolution_dpi);
    let new_canvas = copies.bounds() != dest.bounds();
    prepare_shapes(&mut copies.layers, dest, new_canvas);
    Ok(())
}

/// Moves every layer of `copies` by whole pixels (content, masks, vectors and smart placement).
pub(crate) fn translate_copies(copies: &mut Document, destination: &Document, dx: i32, dy: i32) {
    if dx != 0 || dy != 0 {
        let mut snapshot = copies.clone();
        // Vector caches are clipped and quantized for the destination, not the clipboard's source.
        snapshot.size = destination.size;
        snapshot.mode = destination.mode;
        snapshot.depth = destination.depth;
        snapshot.icc_profile = destination.icc_profile.clone();
        for l in &mut copies.layers {
            crate::commands::translate_layer(&snapshot, l, dx, dy);
            crate::vector_cmds::translate_vectors(&snapshot, l, f64::from(dx), f64::from(dy));
        }
    }
}

/// `layer.copyToDocument`.
fn copy_to_document(s: &mut Session, p: &Value) -> Result<Value> {
    let dest = doc_index(s, p, "document")?.ok_or_else(|| bad("missing `document` (the destination's index)"))?;
    let src = match doc_index(s, p, "source")? {
        Some(i) => i,
        None => s.active_index().ok_or(EngineError::NoDocument)?,
    };
    if src == dest {
        return Err(bad("the destination is the source document (use layer.duplicate)"));
    }
    let (from, to) = (&s.documents()[src], &s.documents()[dest]);
    let (sdoc, ddoc) = (from.doc.clone(), to.doc.clone());
    let ids: Vec<LayerId> = match p.get("layers") {
        None => from.selected_layers(),
        Some(v) => v
            .as_array()
            .map(|a| a.iter().map(|id| id.as_u64().map(LayerId)).collect::<Option<Vec<_>>>())
            .and_then(|ids| ids)
            .ok_or_else(|| bad("`layers` must be an array of layer ids"))?,
    };
    if let Some(missing) = ids.iter().find(|id| sdoc.layer(**id).is_none()) {
        return Err(EngineError::NoLayer(*missing));
    }
    let ids = crate::layer_multi_cmds::top_level(&sdoc, &ids);
    if ids.is_empty() {
        return Err(bad("no layers to copy"));
    }
    for (doc, role) in [(&sdoc, "source"), (&ddoc, "destination")] {
        if !layered(doc.mode) {
            return Err(EngineError::Other(format!("layers can't be copied with a {:?} {role} document", doc.mode)));
        }
    }
    // The copies, bottom to top, in a scratch document of the source's colour so they convert
    // the way a whole document does.
    let mut copies = Document::new("", sdoc.size, sdoc.mode, sdoc.depth);
    copies.icc_profile = sdoc.icc_profile.clone();
    // Type re-lays out at the source's resolution, so it keeps its size in pixels.
    copies.resolution_dpi = sdoc.resolution_dpi;
    for id in &ids {
        let src_layer = sdoc.layer(*id).ok_or(EngineError::NoLayer(*id))?;
        let mut l = src_layer.duplicate();
        // A dragged Background arrives as an ordinary layer (the destination keeps its own).
        if sdoc.is_background(*id) {
            l.name = ddoc.next_layer_name("Layer");
            l.locks = Default::default();
        }
        copies.layers.push(l);
    }
    if let (Some(name), [l]) = (p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()), copies.layers.as_mut_slice()) {
        l.name = name.to_string();
    }
    // Measure in source space: destination refresh may clip a shape before it is moved.
    // Missing caches must participate too, including shapes nested inside groups.
    prepare_shapes(&mut copies.layers, &sdoc, false);
    // What a centred or dropped copy is centred on; the default offset placement doesn't measure.
    let content = if p.get("center").and_then(Value::as_bool).unwrap_or(false) || p.get("at").is_some() {
        let bounds = copies.layers.iter().filter_map(shown_bounds).fold(Rect::EMPTY, |a, b| a.union(&b));
        if bounds.is_empty() { sdoc.bounds() } else { bounds }
    } else {
        Rect::EMPTY
    };
    adapt_copies(s, &mut copies, &ddoc)?;
    // Placement: centred on the destination's canvas, centred on a point, or offset from where
    // the layers are in the source (default: the same canvas position).
    let (dx, dy) = if p.get("center").and_then(Value::as_bool).unwrap_or(false) {
        let (c, d) = (center(content), center(ddoc.bounds()));
        (d[0] - c[0], d[1] - c[1])
    } else if let Some(at) = point(p, "at")? {
        let c = center(content);
        (at[0] - c[0], at[1] - c[1])
    } else {
        point(p, "offset")?.map_or((0.0, 0.0), |o| (o[0], o[1]))
    };
    let (dx, dy) = (dx.round() as i32, dy.round() as i32);
    translate_copies(&mut copies, &ddoc, dx, dy);
    let patterns: Vec<_> = sdoc.patterns.iter().filter(|pat| !ddoc.patterns.iter().any(|d| d.id == pat.id)).cloned().collect();
    s.set_active(dest);
    let label = if ids.len() == 1 { "Duplicate Layer" } else { "Duplicate Layers" };
    let edited = s.edit(label, |doc, active| {
        doc.patterns.extend(patterns);
        let mut above = *active;
        let mut new = Vec::with_capacity(copies.layers.len());
        // The first copy goes where a new layer would; the rest stack above it.
        let mut place = crate::layer_place::Place::AutoEditable;
        for l in copies.layers {
            let id = crate::layer_place::insert_new(doc, above, l, place);
            place = crate::layer_place::Place::Above;
            above = Some(id);
            new.push(id);
        }
        *active = above;
        // Copied groups keep their own depth: repeated copies would otherwise nest without end.
        crate::layer_multi_cmds::check_group_depth(doc, "Copy Layers")?;
        Ok(new)
    });
    // Nothing changed: the source stays the active document.
    if edited.is_err() {
        s.set_active(src);
    }
    let new = edited?;
    let last = new.last().copied();
    crate::layer_multi_cmds::reselect(s, new.clone(), last);
    Ok(json!({"document": dest, "layers": new.iter().map(|l| l.0).collect::<Vec<_>>(), "offset": [dx, dy]}))
}

/// `layer.duplicate` with `"document": "new"`: Photoshop's Duplicate Layer… ›
/// Document: New. A new document the size of the source's canvas holds just the duplicated
/// layers at their canvas positions (a transparent background elsewhere); the source is
/// untouched and the new document becomes active. Mode, depth, resolution, profile,
/// patterns and global light come along so the copies render the same.
pub(crate) fn duplicate_to_new_document(s: &mut Session, p: &Value) -> Result<Value> {
    let src = s.active_index().ok_or(EngineError::NoDocument)?;
    let sdoc = s.documents().get(src).ok_or(EngineError::NoDocument)?.doc.clone();
    let ids = match crate::commands::explicit_layer(s, p)? {
        Some(id) => vec![id],
        None => s.active().ok_or(EngineError::NoDocument)?.selected_layers(),
    };
    if let Some(missing) = ids.iter().find(|id| sdoc.layer(**id).is_none()) {
        return Err(EngineError::NoLayer(*missing));
    }
    let ids = crate::layer_multi_cmds::top_level(&sdoc, &ids);
    if ids.is_empty() {
        return Err(bad("no layers to copy"));
    }
    if !layered(sdoc.mode) {
        return Err(EngineError::Other(format!("layers can't be duplicated with a {:?} document", sdoc.mode)));
    }
    let doc_name = p.get("documentName").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| format!("{} copy", sdoc.name));
    let mut nd = Document::new(doc_name, sdoc.size, sdoc.mode, sdoc.depth);
    nd.resolution_dpi = sdoc.resolution_dpi;
    nd.icc_profile.clone_from(&sdoc.icc_profile);
    nd.patterns.clone_from(&sdoc.patterns);
    nd.global_light = sdoc.global_light;
    for id in &ids {
        let mut l = sdoc.layer(*id).ok_or(EngineError::NoLayer(*id))?.duplicate();
        if ids.len() == 1
            && let Some(name) = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty())
        {
            l.name = name.to_string();
        }
        nd.layers.push(l);
    }
    // Contents ids belong to a document: fresh ones keep the copies independent of the
    // source while preserving sharing within the batch (as `adapt_copies` does).
    let mut contents = HashMap::new();
    let smart_ids: Vec<_> = nd.walk().into_iter().filter_map(|(_, _, l)| matches!(l.content, LayerContent::Smart(_)).then_some(l.id)).collect();
    for id in smart_ids {
        if let Some(l) = nd.layer_mut(id)
            && let LayerContent::Smart(sm) = &mut l.content
        {
            sm.contents_id = *contents.entry(sm.contents_id).or_insert_with(SmartContentsId::fresh);
        }
    }
    let new_ids: Vec<LayerId> = nd.layers.iter().map(|l| l.id).collect();
    let index = s.add_document(nd, None);
    crate::layer_multi_cmds::reselect(s, new_ids.clone(), new_ids.last().copied());
    let mut out = json!({"document": index, "layers": new_ids.iter().map(|l| l.0).collect::<Vec<_>>()});
    if let [one] = new_ids.as_slice()
        && let Some(map) = out.as_object_mut()
    {
        map.insert("layer".into(), json!(one.0));
    }
    Ok(out)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Copy Layers to Document",
        menu: &[],
        shortcut: None,
        params: r##"{"document":index (destination),"layers":[id,…]?=the source's selected layers,"source":index?=active,"center":bool=false (centre on the destination's canvas),"at":[x,y]? (centre the copies on this point),"offset":[dx,dy]?=[0,0] (from their place in the source),"name":str? (one layer: the copy's name)}"##,
        enabled,
        run: copy_to_document,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;
