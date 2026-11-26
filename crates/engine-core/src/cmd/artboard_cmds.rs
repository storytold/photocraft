//! Artboards: Layer › New › Artboard (from scratch, a group or the selected layers), artboard
//! properties (board rect, background, preset), View › Clear Selected Artboard Guides, and
//! File › Export › Artboards to Files / to PDF.
//!
//! An artboard is a top-level group with [`photocraft_doc::Artboard`] set; the compositor clips
//! its children to the board and paints its background. Moving the group (Move tool,
//! `layer.translate`) moves the board with its contents. The canvas grows to fit every board,
//! in every direction, like Photoshop's auto-expanding artboard canvas ([`fit_canvas`]).

use std::collections::HashSet;

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Artboard, ArtboardBackground, Document, Layer, LayerContent, LayerId, Size};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, color_param, int, int_i32, parse_hex};
use crate::file_cmds::{encode, f64_param, join, native_doc, sanitize, stem, str_param, write_file};
use crate::{EngineError, Result, Session};

/// Artboard size presets (name, width, height) in pixels, Photoshop's common devices and pages.
pub const PRESETS: &[(&str, u32, u32)] = &[
    ("iPhone 8/7/6", 750, 1334),
    ("iPhone 14", 390, 844),
    ("iPhone 14 Pro Max", 430, 932),
    ("Android 1080p", 360, 640),
    ("iPad Pro 12.9", 1024, 1366),
    ("Web 1280", 1280, 800),
    ("Web 1366", 1366, 768),
    ("Web 1920", 1920, 1080),
    ("Instagram Post", 1080, 1080),
    ("Letter", 612, 792),
    ("A4", 595, 842),
];

/// Gap Photoshop leaves between a new artboard and the rightmost existing one.
const GAP: i32 = 100;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_artboards(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.has_artboards() { Ok(()) } else { Err("the document has no artboards".into()) }
}

fn export_artboards(s: &Session) -> std::result::Result<(), String> {
    native_doc(s)?;
    has_artboards(s)
}

/// The active layer's artboard (the layer itself or its top-level ancestor).
fn active_artboard(s: &Session) -> Option<LayerId> {
    let d = s.active()?;
    d.doc.artboard_of(d.active_layer?)
}

fn has_active_artboard(s: &Session) -> std::result::Result<(), String> {
    active_artboard(s).map(|_| ()).ok_or_else(|| "select an artboard (or a layer inside one)".into())
}

fn has_plain_group(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    let l = d.doc.layer(id).ok_or("no active layer")?;
    if !l.is_group() || l.artboard().is_some() {
        return Err("select a layer group".into());
    }
    if !d.doc.layers.iter().any(|t| t.id == id) {
        return Err("artboards must be top-level: select a group that is not nested".into());
    }
    Ok(())
}

fn has_free_layers(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let sel = d.selected_layers();
    if sel.is_empty() {
        return Err("select one or more layers".into());
    }
    if sel.iter().any(|id| d.doc.artboard_of(*id).is_some()) {
        return Err("the selection is already in an artboard".into());
    }
    Ok(())
}

fn preset_size(name: &str) -> Option<(u32, u32)> {
    PRESETS.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(name)).map(|(_, w, h)| (*w, *h))
}

fn background_param(p: &Value, cmd: &str) -> Result<Option<ArtboardBackground>> {
    let custom = if p.get("color").is_some() {
        let c = color_param(cmd, p, "color", [1.0; 4])?;
        Some(ArtboardBackground::Custom(Color::rgba(c[0], c[1], c[2], c[3])))
    } else {
        None
    };
    Ok(match p.get("background").and_then(Value::as_str) {
        None => custom,
        Some("white") => Some(ArtboardBackground::White),
        Some("black") => Some(ArtboardBackground::Black),
        Some("transparent") => Some(ArtboardBackground::Transparent),
        Some("custom" | "other") => Some(custom.unwrap_or(ArtboardBackground::White)),
        Some(o) => return Err(bad(cmd, format!("unknown background `{o}` (white|black|transparent|custom)"))),
    })
}

/// A board rect from `x, y, width, height`, or a bad-params error naming them when an edge falls
/// outside the 32-bit coordinate range. `Rect::from_xywh` would clamp the far edge to `i32::MAX`
/// (silently shrinking the board) and wrap a width past `i32::MAX` negative (#931).
fn board_rect(cmd: &str, [x, y, w, h]: [i64; 4]) -> Result<Rect> {
    if w <= 0 || h <= 0 {
        return Err(bad(cmd, format!("the artboard size must be positive (width = {w}, height = {h})")));
    }
    let span = |a: i64, len: i64| Some((i32::try_from(a).ok()?, i32::try_from(a.checked_add(len)?).ok()?));
    match (span(x, w), span(y, h)) {
        (Some((x0, x1)), Some((y0, y1))) => Ok(Rect::new(x0, y0, x1, y1)),
        _ => Err(bad(
            cmd,
            format!("the artboard [x, y, width, height] = [{x}, {y}, {w}, {h}] reaches outside the 32-bit coordinate range ({}..={})", i32::MIN, i32::MAX),
        )),
    }
}

/// `rect` as `[x, y, w, h]` or `x`/`y`/`width`/`height` keys over `base` (`[x, y, w, h]`, wide so
/// a default position past the edge reaches [`board_rect`] instead of overflowing).
fn rect_param(p: &Value, base: [i64; 4], cmd: &str) -> Result<Rect> {
    if let Some(Value::Array(a)) = p.get("rect") {
        let v: Vec<i64> = a.iter().filter_map(|x| x.as_i64().or_else(|| x.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64))).collect();
        let Ok(v) = <[i64; 4]>::try_from(v) else {
            return Err(bad(cmd, "\"rect\" must be [x, y, width, height] with a positive size"));
        };
        return board_rect(cmd, v);
    }
    let x = int_i32(cmd, p, "x")?.map_or(base[0], i64::from);
    let y = int_i32(cmd, p, "y")?.map_or(base[1], i64::from);
    let w = int(p, "width").unwrap_or(base[2]);
    let h = int(p, "height").unwrap_or(base[3]);
    board_rect(cmd, [x, y, w, h])
}

/// `r` as `[x, y, w, h]` for [`rect_param`].
fn xywh(r: Rect) -> [i64; 4] {
    [i64::from(r.x0), i64::from(r.y0), i64::from(r.width()), i64::from(r.height())]
}

/// Layer › Duplicate on an artboard (#1531): Photoshop never stacks the copy on the original,
/// it puts it right of the board, a [`GAP`] away, sliding further right past any board in the
/// way. Moves the copy, layer `id` (just inserted on top of its original), there with its
/// contents and grows the canvas to show it; anything that isn't an artboard is left alone.
/// Returns [`fit_canvas`]'s shift (a board already past the origin, as a PSD can hold, re-bases the
/// document), for [`follow_shift`].
pub(crate) fn place_copy(doc: &mut Document, id: LayerId) -> Result<(i32, i32)> {
    let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let Some(r) = l.artboard().map(|a| a.rect) else { return Ok((0, 0)) };
    let name = l.name.clone();
    let boards: Vec<Rect> = doc.artboards().iter().filter(|b| b.0 != id).map(|b| b.2.rect).collect();
    let (y0, y1, w) = (i64::from(r.y0), i64::from(r.y1), i64::from(r.width()));
    // In i64: `x1 + GAP` passes i32::MAX for a board at the far right edge (#931).
    let mut x = i64::from(r.x1) + i64::from(GAP);
    // Each round clears every board in the way, so one round per board settles it.
    for _ in 0..=boards.len() {
        let blocked =
            boards.iter().filter(|b| i64::from(b.x0) < x + w && i64::from(b.x1) > x && i64::from(b.y0) < y1 && i64::from(b.y1) > y0).map(|b| b.x1).max();
        match blocked {
            Some(edge) => x = i64::from(edge) + i64::from(GAP),
            None => break,
        }
    }
    let fits = x.checked_add(w).is_some_and(|x1| x1 <= i64::from(i32::MAX));
    let dx = i32::try_from(x - i64::from(r.x0)).ok().filter(|_| fits).ok_or_else(|| {
        EngineError::Other(format!("no room for a copy of artboard \"{name}\" right of it; duplicate it in place (\"inPlace\": true) and move it"))
    })?;
    // Grow the canvas first, so shapes on the board re-render uncut where they land.
    let x1 = i64::from(doc.size.width).max(x + w);
    doc.size = Size::new(x1.clamp(1, 300_000) as u32, doc.size.height);
    let snapshot = doc.clone();
    let copy = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    crate::commands::translate_layer(&snapshot, copy, dx, 0);
    crate::vector_cmds::translate_vectors(&snapshot, copy, f64::from(dx), 0.0);
    Ok(fit_canvas(doc))
}

/// Record a Duplicate's `inPlace` only when a copy (the selection after it) is an artboard: for
/// any other layer it changes nothing, and Photoshop's Duplicate can't express it, so a recorded
/// Alt-drag copy of a plain layer must stay exportable to .atn (#7200).
pub(crate) fn recorded_duplicate(s: &Session, mut p: Value) -> Value {
    let Some(st) = s.active() else { return p };
    if st.selected_layers().iter().copied().any(|id| st.doc.layer(id).is_some_and(|l| l.artboard().is_some())) {
        return p;
    }
    if let Some(fields) = p.as_object_mut() {
        fields.remove("inPlace");
    }
    p
}

/// Grow the canvas so every artboard fits. Photoshop's artboard documents grow in every
/// direction: a board left of or above the origin shifts the whole document (boards, their
/// contents, guides, paths…) right/down by its overhang, then the canvas grows to show it. An
/// axis whose boards span more than the largest canvas, or a shift that would push pixels past
/// the 32-bit range, stays unshifted: the board stays partly off-canvas, as one past the size
/// limit does. Returns the shift, for [`follow_shift`]: `translate_doc` moves the document's own
/// geometry, but the session's Color Sampler points and painting symmetry stay behind unless the
/// caller passes it on.
#[must_use = "pass the shift to `follow_shift`, or the Color Samplers and symmetry stay behind (#9302)"]
pub(crate) fn fit_canvas(doc: &mut Document) -> (i32, i32) {
    let max = i64::from(crate::document_preset_cmds::MAX_DIMENSION);
    let boards: Vec<Rect> = doc.artboards().iter().map(|b| b.2.rect).collect();
    // The canvas and every board, per axis (`lo` ≤ 0 is the canvas origin or a board past it).
    let span = |edges: &dyn Fn(&Rect) -> (i32, i32), len: u32| {
        boards.iter().map(edges).fold((0, i64::from(len)), |(lo, hi), (a, b)| (lo.min(i64::from(a)), hi.max(i64::from(b))))
    };
    let (x0, x1) = span(&|r| (r.x0, r.x1), doc.size.width);
    let (y0, y1) = span(&|r| (r.y0, r.y1), doc.size.height);
    let shift = |lo: i64, hi: i64| if hi - lo <= max { i32::try_from(-lo).unwrap_or(0) } else { 0 };
    let (mut sx, mut sy) = (shift(x0, x1), shift(y0, y1));
    // `translate_doc` checks before it moves anything, so a refusal leaves the document as is.
    if (sx, sy) != (0, 0) && crate::image_cmds::translate_doc(doc, "artboard", sx, sy).is_err() {
        (sx, sy) = (0, 0);
    }
    let size = Size::new((x1 + i64::from(sx)).clamp(1, max) as u32, (y1 + i64::from(sy)).clamp(1, max) as u32);
    if size != doc.size {
        doc.size = size;
        // Shape pixels are cut at the canvas: a board moved past the old edge rendered its
        // shapes against it, leaving them missing or clipped (#2389).
        crate::canvas_geom::refresh(doc, crate::canvas_geom::Refresh::Shapes);
    }
    (sx, sy)
}

/// Keep the Color Sampler points and the painting symmetry on their pixels after an edit shifted
/// the document by `(dx, dy)` to show a board past the origin ([`fit_canvas`]), as Canvas Size's
/// sampler map does (#9302). View state: no history step. The canvas only grew, so every point
/// stays on it.
pub(crate) fn follow_shift(s: &mut Session, (dx, dy): (i32, i32)) {
    let Some(st) = s.active_mut().filter(|_| (dx, dy) != (0, 0)) else { return };
    for p in &mut st.color_samplers {
        *p = [p[0] + f64::from(dx), p[1] + f64::from(dy)];
    }
    let a = photocraft_geom::Affine::translate(f64::from(dx), f64::from(dy));
    st.symmetry = st.symmetry.take().and_then(|sym| sym.transform(&a));
}

fn next_artboard_name(doc: &Document) -> String {
    let names: Vec<&str> = doc.artboards().into_iter().map(|b| b.1).collect();
    // `names.len() + 1` candidates always include a free one.
    (1..=names.len() + 1).map(|n| format!("Artboard {n}")).find(|n| !names.contains(&n.as_str())).unwrap_or_else(|| "Artboard".into())
}

/// File › New with Artboards (#9086): one board over the whole canvas, its background the
/// document's (white, black, a colour or transparent), holding an empty Layer 1. As in Photoshop,
/// there is no Background layer.
pub fn new_artboard_document(name: String, size: Size, mode: ColorMode, depth: SampleType, background: Option<Color>) -> Document {
    let mut doc = Document::new(name, size, mode, depth);
    let background = match background {
        None => ArtboardBackground::Transparent,
        Some(c) if c == Color::WHITE => ArtboardBackground::White,
        Some(c) if c == Color::BLACK => ArtboardBackground::Black,
        Some(c) => ArtboardBackground::Custom(c),
    };
    let mut board = Layer::group("Artboard 1", vec![Layer::raster("Layer 1", doc.pixel_format())]);
    set_artboard(&mut board, Artboard { rect: doc.bounds(), background, preset: String::new() });
    doc.layers.push(board);
    doc
}

fn set_artboard(l: &mut Layer, a: Artboard) {
    if let LayerContent::Group(g) = &mut l.content {
        g.artboard = Some(a);
        g.expanded = true;
    }
    // Artboards composite like pass-through groups.
    l.clipped = false;
}

fn new_artboard(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.new.artboard";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let preset = p.get("preset").and_then(Value::as_str).unwrap_or("").to_string();
    let (pw, ph) = match preset.as_str() {
        "" => (d.doc.size.width, d.doc.size.height),
        name => preset_size(name).ok_or_else(|| bad(cmd, format!("unknown preset `{name}`")))?,
    };
    // Default placement: right of the rightmost artboard, else the canvas origin.
    let right = d.doc.artboards().iter().map(|b| b.2.rect.x1).max();
    let top = d.doc.artboards().iter().map(|b| b.2.rect.y0).min().unwrap_or(0);
    // In i64: `x1 + GAP` passes i32::MAX for a board at the far right edge (#931).
    let base = match right {
        Some(x) => [i64::from(x) + i64::from(GAP), i64::from(top), i64::from(pw), i64::from(ph)],
        None => [0, 0, i64::from(pw), i64::from(ph)],
    };
    // With no room right of the rightmost board, ask for a position rather than pick another
    // spot that may overlap a board; an explicit `x` (or `rect`) still works.
    let explicit_x = p.get("rect").is_some() || p.get("x").is_some_and(|v| !v.is_null());
    if let Some(edge) = right.filter(|_| !explicit_x && base[0] > i64::from(i32::MAX)) {
        return Err(bad(cmd, format!("no room for a new artboard right of the rightmost one (its right edge is x = {edge}); pass \"x\" to place it")));
    }
    // A preset decides the size (dialogs send the canvas size alongside it).
    let rect = if preset.is_empty() || p.get("rect").is_some() {
        rect_param(p, base, cmd)?
    } else {
        rect_param(&json!({"x": p.get("x"), "y": p.get("y")}), base, cmd)?
    };
    let background = background_param(p, cmd)?.unwrap_or_default();
    let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
    let take_background = match p.get("takeBackground") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "`takeBackground` must be true or false"))?,
    };
    let (id, shift) = s.edit("New Artboard", |doc, active| {
        let mut g = Layer::group(name.unwrap_or_else(|| next_artboard_name(doc)), vec![]);
        set_artboard(&mut g, Artboard { rect, background, preset });
        // Only the first board takes the Background in (decided before the board exists).
        let take = take_background && doc.artboards().is_empty() && doc.background().is_some();
        let id = doc.insert_above(None, g);
        if take {
            move_background_into(doc, id);
        }
        let shift = fit_canvas(doc);
        *active = Some(id);
        Ok((id, shift))
    })?;
    follow_shift(s, shift);
    crate::layer_multi_cmds::set_selection(s, vec![id], Some(id), Some(id))?;
    Ok(json!({"layer": id.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

/// Turns the Background into a normal layer ("Layer 0") and makes it the child of board `board`,
/// as the reference does when the first artboard is drawn. Does nothing without a Background.
fn move_background_into(doc: &mut Document, board: LayerId) {
    let Some(bg) = doc.background().map(|l| l.id) else { return };
    let Some(mut layer) = doc.remove(bg) else { return };
    crate::extra_cmds::unlock_background(&mut layer);
    match doc.layer_mut(board).and_then(Layer::children_mut) {
        Some(children) => children.push(layer),
        // Unreachable for a group; put the layer back rather than lose it.
        None => doc.layers.insert(0, layer),
    }
}

/// Convert a plain group at its content bounds; recorded guide IDs are ignored session metadata.
fn artboard_from_group(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.new.artboardFromGroup";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = crate::commands::layer_param(s, p)?;
    let preset = match p.get("preset") {
        None => String::new(),
        Some(value) => value.as_str().ok_or_else(|| bad(cmd, "preset must be text"))?.to_string(),
    };
    let apply_background = match p.get("changeBackground") {
        None | Some(Value::Bool(true)) => true,
        Some(value) => value.as_i64().and_then(|n| i32::try_from(n).ok()).ok_or_else(|| bad(cmd, "changeBackground must be an integer"))? == 1,
    };
    if let Some(guides) = p.get("guideIDs")
        && !guides.as_array().is_some_and(|ids| ids.iter().all(|id| id.as_i64().is_some_and(|n| i32::try_from(n).is_ok())))
    {
        return Err(bad(cmd, "guideIDs must be a list of integers"));
    }
    let canvas = d.doc.bounds();
    let background = if apply_background { background_param(p, cmd)?.unwrap_or_default() } else { ArtboardBackground::White };
    let (rect, shift) = s.edit("Artboard from Group", |doc, active| {
        if !doc.layers.iter().any(|t| t.id == id && t.is_group() && t.artboard().is_none()) {
            return Err(EngineError::Other("artboards are made from a top-level layer group".into()));
        }
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        // A hidden group keeps today's measure (not measured in the reference).
        let rect = if l.visible { shown_children_bounds(l) } else { crate::layer_multi_cmds::layer_bounds(l) }.unwrap_or(canvas);
        set_artboard(l, Artboard { rect, background, preset });
        // As in the reference application, the new artboard is Normal at 100 % whatever the group had.
        l.blend = photocraft_color::BlendMode::Normal;
        l.opacity = 1.0;
        let shift = fit_canvas(doc);
        *active = Some(id);
        Ok((rect, shift))
    })?;
    follow_shift(s, shift);
    crate::layer_multi_cmds::set_selection(s, vec![id], Some(id), Some(id))?;
    Ok(json!({"layer": id.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

/// [`layer_multi_cmds::layer_bounds`](crate::layer_multi_cmds::layer_bounds) of what shows: a
/// group unions only its visible children, so hidden layers (and a hidden group's children) don't
/// size the board Artboard from Group makes (#14553).
fn shown_children_bounds(l: &Layer) -> Option<Rect> {
    match l.children() {
        Some(children) => children.iter().filter(|c| c.visible).filter_map(shown_children_bounds).reduce(|a, b| a.union(&b)),
        None => crate::layer_multi_cmds::layer_bounds(l),
    }
}

/// The layers' union bounds, or the canvas when they have none: the board Artboard from Layers
/// makes when no size is given.
fn union_bounds<'a>(layers: impl IntoIterator<Item = &'a Layer>, canvas: Rect) -> Rect {
    layers.into_iter().filter_map(crate::layer_multi_cmds::layer_bounds).reduce(|a, b| a.union(&b)).unwrap_or(canvas)
}

/// What Artboard from Layers' dialog opens with, as in the reference application: the name the
/// board would get and the selected top-level layers' size (`None` without a document).
pub fn from_layers_defaults(s: &Session) -> Option<(String, [i64; 2])> {
    let d = s.active()?;
    let ids = crate::layer_multi_cmds::top_level(&d.doc, &crate::layer_multi_cmds::selected(s));
    let [_, _, w, h] = xywh(union_bounds(ids.iter().filter_map(|id| d.doc.layer(*id)), d.doc.bounds()));
    Some((next_artboard_name(&d.doc), [w, h]))
}

fn artboard_from_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let sel = crate::layer_multi_cmds::selected(s);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
    let cmd = "layer.new.artboardFromLayers";
    let background = background_param(p, cmd)?.unwrap_or_default();
    // `int` rounds, so a small negative (-0.4) would pass as 0, "keep the bounds".
    if ["width", "height"].iter().any(|k| p.get(*k).and_then(Value::as_f64).is_some_and(|v| v < 0.0)) {
        return Err(bad(cmd, "the artboard width and height must not be negative"));
    }
    // A size of the user's own (a post, a story…) from the layers' top left; 0 or none keeps
    // their bounds on that axis (#9087).
    let size = [int(p, "width").filter(|v| *v != 0), int(p, "height").filter(|v| *v != 0)];
    let (gid, rect, shift) = s.edit("New Artboard From Layers", |doc, active| {
        let ids = crate::layer_multi_cmds::top_level(doc, &sel);
        if ids.is_empty() || ids.iter().any(|id| doc.artboard_of(*id).is_some()) {
            return Err(EngineError::Other("select layers that are not in an artboard".into()));
        }
        let mut children = Vec::with_capacity(ids.len());
        for id in &ids {
            children.push(doc.remove(*id).ok_or(EngineError::NoLayer(*id))?);
        }
        let [x, y, w, h] = xywh(union_bounds(&children, canvas));
        let rect = board_rect(cmd, [x, y, size[0].unwrap_or(w), size[1].unwrap_or(h)])?;
        let mut g = Layer::group(name.unwrap_or_else(|| next_artboard_name(doc)), children);
        set_artboard(&mut g, Artboard { rect, background, preset: String::new() });
        let gid = doc.insert_above(None, g);
        crate::layer_multi_cmds::check_group_depth(doc, "Artboard from Layers")?;
        let shift = fit_canvas(doc);
        *active = Some(gid);
        Ok((gid, rect, shift))
    })?;
    follow_shift(s, shift);
    crate::layer_multi_cmds::set_selection(s, vec![gid], Some(gid), Some(gid))?;
    Ok(json!({"layer": gid.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

/// Properties panel / Artboard tool: board position and size, background, preset, name.
/// Changing X/Y moves the board with its contents; changing W/H resizes it from the top left.
fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.set";
    if p.get("layers").is_some() {
        return set_backgrounds(s, p);
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = crate::commands::explicit_layer(s, p)?.or_else(|| active_artboard(s)).ok_or_else(|| bad(cmd, "no artboard selected"))?;
    let old = d.doc.layer(id).and_then(Layer::artboard).cloned().ok_or_else(|| bad(cmd, "the layer is not an artboard"))?;
    let preset = p.get("preset").and_then(Value::as_str).map(str::to_string);
    let mut base = xywh(old.rect);
    if let Some(name) = preset.as_deref().filter(|n| !n.is_empty()) {
        let (w, h) = preset_size(name).ok_or_else(|| bad(cmd, format!("unknown preset `{name}`")))?;
        (base[2], base[3]) = (i64::from(w), i64::from(h));
    }
    let rect = rect_param(p, base, cmd)?;
    let background = background_param(p, cmd)?;
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let move_contents = p.get("moveContents").and_then(Value::as_bool).unwrap_or(true);
    // The offset between two in-range edges can need 33 bits (#931); layers move by i32 only.
    let (dx, dy) = (i64::from(rect.x0) - i64::from(old.rect.x0), i64::from(rect.y0) - i64::from(old.rect.y0));
    let (dx, dy) = match (i32::try_from(dx), i32::try_from(dy)) {
        (Ok(dx), Ok(dy)) => (dx, dy),
        _ if !move_contents => (0, 0),
        _ => {
            return Err(bad(
                cmd,
                format!(
                    "moving the artboard with its contents by ({dx}, {dy}) is outside the 32-bit range; move it in smaller steps or pass \"moveContents\": false"
                ),
            ));
        }
    };
    let shift = s.edit("Edit Artboard", |doc, _| {
        if move_contents && (dx, dy) != (0, 0) {
            let snapshot = doc.clone();
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            crate::commands::translate_layer(&snapshot, l, dx, dy);
            crate::vector_cmds::translate_vectors(&snapshot, l, f64::from(dx), f64::from(dy));
        }
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if let Some(n) = name {
            l.name = n;
        }
        let a = l.artboard_mut().ok_or(EngineError::NoLayer(id))?;
        a.rect = rect;
        if let Some(b) = background {
            a.background = b;
        }
        match preset {
            Some(pr) => a.preset = pr,
            // A hand-edited size no longer matches the preset.
            None if rect.width() != old.rect.width() || rect.height() != old.rect.height() => a.preset.clear(),
            None => {}
        }
        Ok(fit_canvas(doc))
    })?;
    follow_shift(s, shift);
    Ok(json!({"layer": id.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

/// `layer.artboard.set` with a `layers` array: the background of every listed board in one undo
/// step, "Edit Artboard" for one board and "Edit Artboards" for several (the options bar's
/// background control with boards selected). Only `background`/`color` apply to several boards.
fn set_backgrounds(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.set";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let mut ids: Vec<LayerId> = Vec::new();
    for v in p.get("layers").and_then(Value::as_array).ok_or_else(|| bad(cmd, "`layers` must be an array of layer ids"))? {
        let id = LayerId(v.as_u64().ok_or_else(|| bad(cmd, "`layers` must be an array of layer ids"))?);
        d.doc.layer(id).and_then(Layer::artboard).ok_or_else(|| bad(cmd, format!("layer {} is not an artboard", id.0)))?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(bad(cmd, "`layers` is empty"));
    }
    let background = background_param(p, cmd)?.ok_or_else(|| bad(cmd, "`layers` needs a `background`"))?;
    let label = if ids.len() == 1 { "Edit Artboard" } else { "Edit Artboards" };
    s.edit(label, |doc, _| {
        for id in &ids {
            doc.layer_mut(*id).and_then(Layer::artboard_mut).ok_or(EngineError::NoLayer(*id))?.background = background;
        }
        Ok(())
    })?;
    Ok(json!({"layers": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

/// The Artboard tool's "+" buttons: a board the size of board `layer` on one of its sides, a
/// [`GAP`] away (further out past any board in the way), empty or (`duplicate`, the ⌥-click) a
/// copy with its contents. A board left of or above the canvas origin shifts the whole document
/// right/down so the canvas can grow to show it. One undo step.
fn add_beside(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.artboard.addBeside";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = crate::commands::explicit_layer(s, p)?.or_else(|| active_artboard(s)).ok_or_else(|| bad(cmd, "no artboard selected"))?;
    let src = d.doc.layer(id).and_then(Layer::artboard).cloned().ok_or_else(|| bad(cmd, "the layer is not an artboard"))?;
    let side = p.get("side").and_then(Value::as_str).unwrap_or("right");
    let duplicate = p.get("duplicate").and_then(Value::as_bool).unwrap_or(false);
    let [x0, y0, w, h] = xywh(src.rect);
    let gap = i64::from(GAP);
    // In i64: edges ± a board and a gap can pass the i32 range (#931); `board_rect` checks.
    let (mut x, mut y) = match side {
        "right" => (x0 + w + gap, y0),
        "left" => (x0 - gap - w, y0),
        "bottom" => (x0, y0 + h + gap),
        "top" => (x0, y0 - gap - h),
        o => return Err(bad(cmd, format!("unknown side `{o}` (left|right|top|bottom)"))),
    };
    let boards: Vec<Rect> = d.doc.artboards().iter().map(|b| b.2.rect).collect();
    // Each round clears every board in the way, so one round per board settles it.
    for _ in 0..=boards.len() {
        let blocked: Vec<&Rect> =
            boards.iter().filter(|b| i64::from(b.x0) < x + w && i64::from(b.x1) > x && i64::from(b.y0) < y + h && i64::from(b.y1) > y).collect();
        if blocked.is_empty() {
            break;
        }
        match side {
            "right" => x = blocked.iter().map(|b| i64::from(b.x1)).max().unwrap_or(x) + gap,
            "left" => x = blocked.iter().map(|b| i64::from(b.x0)).min().unwrap_or(x) - gap - w,
            "bottom" => y = blocked.iter().map(|b| i64::from(b.y1)).max().unwrap_or(y) + gap,
            _ => y = blocked.iter().map(|b| i64::from(b.y0)).min().unwrap_or(y) - gap - h,
        }
    }
    let rect = board_rect(cmd, [x, y, w, h])?;
    // The canvas starts at (0, 0): a board past it moves everything right/down instead.
    let (sx, sy) = (rect.x0.min(0).checked_neg(), rect.y0.min(0).checked_neg());
    let (Some(sx), Some(sy)) = (sx, sy) else {
        return Err(bad(cmd, format!("no room for an artboard {side} of \"{}\"", d.doc.layer(id).map_or("", |l| l.name.as_str()))));
    };
    let grown = |len: u32, by: i32| (i64::from(len) + i64::from(by)).clamp(1, 300_000) as u32;
    let label = if duplicate { "Duplicate Artboard" } else { "New Artboard" };
    let add_copy = crate::commands::layer_panel_option(s, "addCopyToCopiedLayersAndGroups", true);
    let (nid, rect, (fx, fy)) = s.edit(label, |doc, active| {
        if (sx, sy) != (0, 0) {
            crate::image_cmds::translate_doc(doc, cmd, sx, sy)?;
            doc.size = Size::new(grown(doc.size.width, sx), grown(doc.size.height, sy));
            crate::canvas_geom::refresh(doc, crate::canvas_geom::Refresh::Shapes);
        }
        let (Some(x0), Some(y0), Some(x1), Some(y1)) = (rect.x0.checked_add(sx), rect.y0.checked_add(sy), rect.x1.checked_add(sx), rect.y1.checked_add(sy))
        else {
            return Err(bad(cmd, "the new artboard is outside the 32-bit coordinate range"));
        };
        let rect = Rect::new(x0, y0, x1, y1);
        let nid = if duplicate {
            let src_rect = doc.layer(id).and_then(Layer::artboard).map(|a| a.rect).ok_or(EngineError::NoLayer(id))?;
            let (dx, dy) = (rect.x0.saturating_sub(src_rect.x0), rect.y0.saturating_sub(src_rect.y0));
            let copy = crate::commands::layer_copy(doc, id, add_copy)?;
            let nid = doc.insert_above(Some(id), copy);
            // Grow the canvas first, so shapes on the board re-render uncut where they land.
            let fit = |len: u32, edge: i32| i64::from(len).max(i64::from(edge)).clamp(1, 300_000) as u32;
            doc.size = Size::new(fit(doc.size.width, rect.x1), fit(doc.size.height, rect.y1));
            let snapshot = doc.clone();
            let l = doc.layer_mut(nid).ok_or(EngineError::NoLayer(nid))?;
            crate::commands::translate_layer(&snapshot, l, dx, dy);
            crate::vector_cmds::translate_vectors(&snapshot, l, f64::from(dx), f64::from(dy));
            nid
        } else {
            let mut g = Layer::group(next_artboard_name(doc), vec![]);
            set_artboard(&mut g, Artboard { rect, background: src.background, preset: src.preset.clone() });
            doc.insert_above(Some(id), g)
        };
        let fitted = fit_canvas(doc);
        *active = Some(nid);
        Ok((nid, rect, fitted))
    })?;
    follow_shift(s, (sx.saturating_add(fx), sy.saturating_add(fy)));
    crate::layer_multi_cmds::set_selection(s, vec![nid], Some(nid), Some(nid))?;
    Ok(json!({"layer": nid.0, "rect": [rect.x0, rect.y0, rect.width(), rect.height()]}))
}

fn clear_artboard_guides(s: &mut Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = active_artboard(s).ok_or(EngineError::Other("no artboard selected".into()))?;
    let r = d.doc.layer(id).and_then(Layer::artboard).map(|a| a.rect).ok_or(EngineError::NoLayer(id))?;
    let inside_x = |x: &f64| *x > f64::from(r.x0) && *x < f64::from(r.x1);
    let inside_y = |y: &f64| *y > f64::from(r.y0) && *y < f64::from(r.y1);
    let n = d.doc.guides.vertical.iter().filter(|x| inside_x(x)).count() + d.doc.guides.horizontal.iter().filter(|y| inside_y(y)).count();
    if n == 0 {
        return Ok(json!({"cleared": 0}));
    }
    s.edit("Clear Selected Artboard Guides", |doc, _| {
        doc.guides.vertical.retain(|x| !inside_x(x));
        doc.guides.horizontal.retain(|y| !inside_y(y));
        doc.guides.prune_colors();
        Ok(())
    })?;
    Ok(json!({"cleared": n}))
}

/// One artboard as its own document: the board's size, its group moved to the origin. `None`
/// when `id` is not an artboard; an error when the board is too far from the origin to move.
pub fn artboard_document(doc: &Document, id: LayerId) -> Result<Option<Document>> {
    let Some(l) = doc.layer(id) else { return Ok(None) };
    let Some(r) = l.artboard().map(|a| a.rect) else { return Ok(None) };
    // A board starting at i32::MIN needs an offset of 2^31 to reach the origin, one past
    // i32::MAX (#931).
    let (Some(dx), Some(dy)) = (r.x0.checked_neg(), r.y0.checked_neg()) else {
        return Err(EngineError::Other(format!("artboard `{}` at ({}, {}) is too far from the origin to export", l.name, r.x0, r.y0)));
    };
    let mut group = l.clone();
    group.visible = true;
    crate::commands::translate_layer(doc, &mut group, dx, dy);
    crate::vector_cmds::translate_vectors(doc, &mut group, f64::from(dx), f64::from(dy));
    let mut one = doc.clone();
    one.name = l.name.clone();
    one.size = Size::new(r.width(), r.height());
    one.layers = vec![group].into();
    one.channels.clear();
    one.selection = None;
    one.quick_mask = None;
    one.layer_comps.clear();
    one.last_applied_comp = None;
    one.last_document_state = None;
    // Paths, notes, slices and Count/Ruler marks move with the board. This also shifts
    // `one.guides`, but they are rebuilt below from the source's, so nothing moves twice.
    crate::canvas_geom::transform_doc_marks(&mut one, &Affine::translate(f64::from(dx), f64::from(dy)));
    let shift = |v: &[f64], o: i32, len: u32| v.iter().map(|g| g - f64::from(o)).filter(|g| *g > 0.0 && *g < f64::from(len)).collect::<Vec<_>>();
    one.guides.vertical = shift(&doc.guides.vertical, r.x0, r.width());
    one.guides.horizontal = shift(&doc.guides.horizontal, r.y0, r.height());
    Ok(Some(one))
}

/// Include Overlapping Areas: each visible artboard stacked above board `id` that overlaps it, as
/// the reference application writes it into the board's file: a plain group (no longer an
/// artboard) holding its layers over a new pixel layer "Background for <board>" filled with the
/// board's background where the two boards overlap, in source coordinates and bottom to top. A
/// group whose layers reach past its own board (effects, pixels outside the board, adjustments)
/// gets a hard mask to its board, which the artboard clipped them to.
///
/// The artboard painted its background beneath the group and then composited the group (its
/// blend mode, opacity, effects and masks) over it. A board with any of those keeps them on a
/// group of its own inside the plain group, above the background, so they still apply to its
/// layers only and its own mask keeps its feather and Layer Mask Hides Effects setting; the plain
/// group then carries nothing but the board clip.
fn overlapping_boards(doc: &Document, id: LayerId, rect: Rect) -> Vec<Layer> {
    let above = doc.layers.iter().skip_while(|l| l.id != id).skip(1);
    let mut out = Vec::new();
    for l in above.filter(|l| l.visible) {
        let Some(ab) = l.artboard().cloned() else { continue };
        let overlap = ab.rect.intersect(&rect);
        if overlap.is_empty() {
            continue;
        }
        let mut board = l.clone();
        // The board's own shadow or glow is built from all its layers, so they stay whole.
        let fx = photocraft_compose::effects::has_effects(&board);
        if let LayerContent::Group(g) = &mut board.content {
            g.artboard = None;
            if !fx {
                cut_pixels(&mut g.children, overlap, 0);
            }
        }
        let bg = ab.background.rgba();
        let mut group = if stacks_like_its_layers(&board) {
            board
        } else {
            board.clipped = false;
            let mut plain = Layer::group(l.name.clone(), vec![board]);
            plain.label = l.label;
            plain
        };
        if let (Some(bg), Some(children)) = (bg, group.children_mut()) {
            let fmt = doc.pixel_format();
            let mut surf = Surface::new(fmt);
            surf.fill_rect(overlap, &photocraft_raster::from_rgba(&fmt, bg));
            children.insert(0, Layer::new(format!("Background for {}", l.name), LayerContent::Raster(surf)));
        }
        // With the background in place: its pixels widen what the group's layers can reach.
        let reach = photocraft_compose::change_bounds(&group, doc.bounds());
        if reach.is_none_or(|r| !ab.rect.contains_rect(&r)) {
            let mut mask = photocraft_doc::LayerMask::reveal_all();
            mask.surface = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
            mask.surface.fill_rect(ab.rect, &[1.0]);
            group.mask = Some(mask);
        }
        out.push(group);
    }
    out
}

/// Whether artboard `board` (already a plain group) composites over its background exactly as
/// its layers stacked over that background in one pass-through group do: pass-through (a Normal
/// group renders its layers on their own first, so a Multiply or Screen layer in it never blends
/// with the background), and no effects, masks, opacity, Blend If or other blending options of
/// its own, nor a mask the board clip would replace.
fn stacks_like_its_layers(board: &Layer) -> bool {
    board.blend == photocraft_color::BlendMode::PassThrough
        && board.opacity >= 1.0
        && board.fill_opacity >= 1.0
        && board.mask.is_none()
        && board.vector_mask.is_none()
        && board.frame.is_none()
        && board.excluded_channels == 0
        && board.blend_if == photocraft_doc::BlendIf::default()
        && board.advanced == photocraft_doc::AdvancedBlending::default()
        && !photocraft_compose::effects::has_effects(board)
}

/// Cuts the pixel layers in `layers` (and in groups inside them, `depth` levels down so far) to
/// `keep`, as the reference application cuts an overlapping board's layers to the file. Layers
/// whose pixels can't be cut stay whole; the group's mask still hides what lies past its board.
/// A layer with effects, or inside a group with effects, stays whole too: pixels outside `keep`
/// can cast a shadow or glow into it.
fn cut_pixels(layers: &mut [Layer], keep: Rect, depth: usize) {
    const MAX_DEPTH: usize = 64;
    for l in layers {
        if photocraft_compose::effects::has_effects(l) {
            continue;
        }
        match &mut l.content {
            LayerContent::Group(g) if depth < MAX_DEPTH => cut_pixels(&mut g.children, keep, depth + 1),
            LayerContent::Raster(surf) => {
                let r = surf.content_bounds().intersect(&keep);
                let mut cut = Surface::with_default(surf.format(), &surf.default_pixel());
                if !r.is_empty() {
                    let Ok(px) = surf.try_read_region(r) else { continue };
                    if cut.try_write_region(r, &px).is_err() {
                        continue;
                    }
                }
                *surf = cut;
            }
            _ => {}
        }
    }
}

/// [`artboard_document`] for each chosen board, all built before any file is written so a
/// board that can't be exported fails the export without leaving a partial set behind. With
/// `overlapping`, each file also holds the boards above that overlap it ([`overlapping_boards`]).
fn chosen_documents(doc: &Document, p: &Value, overlapping: bool) -> Result<Vec<Document>> {
    Ok(chosen_board_documents(doc, p, overlapping)?.into_iter().map(|(_, one)| one).collect())
}

/// [`chosen_documents`], each with the id of the board it was built from.
fn chosen_board_documents(doc: &Document, p: &Value, overlapping: bool) -> Result<Vec<(LayerId, Document)>> {
    let mut out = Vec::new();
    for id in chosen_artboards(doc, p) {
        let Some(mut one) = artboard_document(doc, id)? else { continue };
        if overlapping && let Some(r) = doc.layer(id).and_then(|l| l.artboard()).map(|a| a.rect) {
            // `artboard_document` has checked that the board's origin negates.
            let (dx, dy) = (r.x0.saturating_neg(), r.y0.saturating_neg());
            for mut group in overlapping_boards(doc, id, r) {
                crate::commands::translate_layer(doc, &mut group, dx, dy);
                crate::vector_cmds::translate_vectors(doc, &mut group, f64::from(dx), f64::from(dy));
                one.layers.push(group);
            }
        }
        out.push((id, one));
    }
    Ok(out)
}

/// `"content"`: `"overlapping"` (Include Overlapping Areas, the default) or `"artboard"` (Artboard
/// Content Only).
fn overlapping_param(p: &Value, cmd: &str) -> Result<bool> {
    match p.get("content") {
        None | Some(Value::Null) => Ok(true),
        Some(v) => match v.as_str() {
            Some("overlapping") => Ok(true),
            Some("artboard") => Ok(false),
            _ => Err(bad(cmd, format!("`content` must be \"overlapping\" or \"artboard\", not {v}"))),
        },
    }
}

/// The artboards an export covers: `"artboards": [ids]`, else all, top of the Layers panel first.
fn chosen_artboards(doc: &Document, p: &Value) -> Vec<LayerId> {
    let all: Vec<LayerId> = doc.artboards().into_iter().rev().map(|b| b.0).collect();
    match p.get("artboards") {
        Some(Value::Array(a)) => {
            let want: Vec<u64> = a.iter().filter_map(Value::as_u64).collect();
            all.into_iter().filter(|id| want.contains(&id.0)).collect()
        }
        _ => all,
    }
}

/// Artboards to Files' Include Artboard Name and its Name Options (#13637).
struct NameBorder {
    /// `{"font", "fontStyle", "size", "color"}` for [`crate::type_cmds::apply_char_props`].
    style: Value,
    /// The Canvas Extension colour, sRGB with alpha.
    extension: [f32; 4],
}

/// Photoshop's largest type size, in points.
const NAME_SIZE_MAX_PT: f64 = 1296.0;

/// The Name Options of `file.export.artboardsToFiles`, or `None` with `includeName` off. A size
/// that is not a positive number fails the export, as in Photoshop ("the initial bounding
/// rectangle is empty"), before any file is written.
fn name_border_param(p: &Value, cmd: &str) -> Result<Option<NameBorder>> {
    if !p.get("includeName").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(None);
    }
    // The dialog's Size field takes any text, so a numeric string counts as its number.
    let size = match p.get("fontSize") {
        None | Some(Value::Null) => Some(12.0),
        Some(Value::String(t)) => t.trim().parse::<f64>().ok(),
        Some(v) => v.as_f64(),
    };
    let Some(size) = size.filter(|v| v.is_finite() && *v > 0.0) else {
        return Err(bad(cmd, "the artboard name size must be a positive number of points"));
    };
    let font = p.get("font").and_then(Value::as_str).filter(|f| !f.trim().is_empty()).unwrap_or("Arial");
    let font_style = p.get("fontStyle").and_then(Value::as_str).filter(|f| !f.trim().is_empty()).unwrap_or("Regular");
    let white = [1.0, 1.0, 1.0, 1.0];
    let extension = match p.get("extension") {
        None | Some(Value::Null) => white,
        Some(Value::String(c)) if c.eq_ignore_ascii_case("white") => white,
        Some(Value::String(c)) if c.eq_ignore_ascii_case("black") => [0.0, 0.0, 0.0, 1.0],
        // The dialog's Other...: the colour picked in its swatch.
        Some(Value::String(c)) if c.eq_ignore_ascii_case("other") => color_param(cmd, p, "extensionColor", white)?,
        Some(Value::String(c)) => parse_hex(c).ok_or_else(|| bad(cmd, "`extension` must be \"white\", \"black\", \"#rrggbb\" or [r, g, b]"))?,
        Some(_) => color_param(cmd, p, "extension", white)?,
    };
    let text = color_param(cmd, p, "textColor", [0.0, 0.0, 0.0, 1.0])?.map(|c| c.clamp(0.0, 1.0));
    let style = json!({"font": font, "fontStyle": font_style, "size": size.min(NAME_SIZE_MAX_PT), "color": text});
    Ok(Some(NameBorder { style, extension: extension.map(|c| c.clamp(0.0, 1.0)) }))
}

/// The width of the name border on each side: a twentieth of width plus height, rounded. This
/// gives exactly Photoshop's measured borders: 18 px for 200 x 150, 20 for 200 x 200 and 60 for
/// 800 x 400.
fn name_border_width(w: u32, h: u32) -> u32 {
    ((f64::from(w) + f64::from(h)) / 20.0).round() as u32
}

/// Include Artboard Name: extends `one` on all four sides by [`name_border_width`], fills the new
/// border with the Canvas Extension colour (a bottom layer, so the board keeps its own pixels)
/// and draws the board's name in the top border, its ink left-aligned with the board's left edge
/// and centred in the border's height.
fn add_name_border(one: &mut Document, o: &NameBorder, cmd: &str) -> Result<()> {
    let (w, h) = (one.size.width, one.size.height);
    let b = name_border_width(w, h);
    let grown = |len: u32| b.checked_mul(2).and_then(|d| len.checked_add(d)).and_then(|n| i32::try_from(n).ok().map(|i| (n, i)));
    let (Some((nw, nwi)), Some((nh, nhi)), Ok(bi)) = (grown(w), grown(h), i32::try_from(b)) else {
        return Err(bad(cmd, format!("artboard `{}` is too large to add a name border", one.name)));
    };
    let before = one.clone();
    for l in &mut one.layers {
        crate::commands::translate_layer(&before, l, bi, bi);
        crate::vector_cmds::translate_vectors(&before, l, f64::from(bi), f64::from(bi));
    }
    crate::canvas_geom::transform_doc_marks(one, &Affine::translate(f64::from(bi), f64::from(bi)));
    one.size = Size::new(nw, nh);
    let fmt = one.pixel_format();
    let mut ring = Layer::raster("Canvas Extension", fmt);
    if let Some(surf) = ring.surface_mut() {
        let px = photocraft_raster::from_rgba(&fmt, o.extension);
        let (x1, y1) = (nwi - bi, nhi - bi);
        for r in [Rect::new(0, 0, nwi, bi), Rect::new(0, y1, nwi, nhi), Rect::new(0, bi, bi, y1), Rect::new(x1, bi, nwi, y1)] {
            if !r.is_empty() {
                surf.fill_rect(r, &px);
            }
        }
    }
    one.layers.insert(0, ring);
    let mut style = photocraft_doc::text::CharStyle::default();
    crate::type_cmds::apply_char_props(&mut style, &o.style);
    let mut t = photocraft_doc::TextLayer { text: one.name.clone(), ..Default::default() };
    t.runs = vec![photocraft_doc::text::TextRun { len: t.text.len(), style }];
    t.sync_summary();
    let dpi = one.resolution_dpi;
    let layout = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).layout(&t, dpi);
    if let Some([x0, y0, _, y1]) = photocraft_text::render::ink_bounds(&layout) {
        let (dx, dy) = ((f64::from(bi) - x0).round(), (f64::from(bi) / 2.0 - (y0 + y1) / 2.0).round());
        if dx.is_finite() && dy.is_finite() {
            t.transform = Affine::translate(dx, dy);
            crate::type_cmds::refresh_at(dpi, fmt, &mut t);
            // Only the top border takes the name: ink of a size taller than the border is cut
            // there, never drawn over the board (the file is flat, so the type need not stay
            // editable).
            if let Some(cache) = &t.cache {
                let r = cache.content_bounds().intersect(&Rect::new(0, 0, nwi, bi));
                if !r.is_empty() {
                    let px = cache.try_read_region(r).map_err(|e| bad(cmd, e.to_string()))?;
                    let mut clipped = Surface::new(cache.format());
                    clipped.write_region(r, &px);
                    one.layers.push(Layer::new(t.text.clone(), LayerContent::Raster(clipped)));
                }
            }
        }
    }
    Ok(())
}

/// File types Include Artboard Name leaves as they are: what it does there is not measured.
fn keeps_layers(format: &str) -> bool {
    matches!(format, "psd" | "psb" | "pdf" | "pcraft")
}

/// What an Artboards to Files `format` writes (the dialog's File Type list).
enum FileType {
    /// One file per board through the format's encoder, with this extension.
    Save(String),
    /// One indexed (palette) PNG per board, as PNG-8.
    Png8,
    /// One truecolour PNG per board, as PNG-24: a Grayscale or other non-RGB board becomes RGB.
    Png24,
    /// One multi-page PDF, a page per board, bottom artboard first.
    Pdf,
}

impl FileType {
    /// Any other token is an extension for the encoder to pick by. It must be one plain extension:
    /// `unknown.png` would name `<board>.unknown.png` and write a PNG, and a `/` would leave `dir`.
    fn of(format: &str, cmd: &str) -> Result<Self> {
        Ok(match format {
            "png8" | "png-8" => FileType::Png8,
            "png24" | "png-24" => FileType::Png24,
            "targa" => FileType::Save("tga".into()),
            "pdf" => FileType::Pdf,
            ext if !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_alphanumeric()) => FileType::Save(ext.into()),
            other => return Err(bad(cmd, format!("unknown file type `{other}`"))),
        })
    }
}

/// The formats Include Background in Export leaves the fill out of: those that keep transparency.
const TRANSPARENT_FORMATS: &[&str] = &["png", "tif", "tiff", "webp"];

/// Whether Artboards to Files keeps the boards' fill: yes unless `"includeBackground": false` asks
/// otherwise for PNG (PNG-24 included), TIFF or WebP. PSD, PNG-8, PDF and the formats without
/// alpha (JPEG, BMP, Targa) keep today's file, as #13631 asks: the reference application was not
/// measured there. [`drop_white_fill`] then leaves out only what was measured.
fn includes_background(p: &Value, file_type: &FileType) -> bool {
    let keeps_alpha = match file_type {
        FileType::Png24 => true,
        FileType::Save(ext) => TRANSPARENT_FORMATS.contains(&ext.as_str()),
        FileType::Png8 | FileType::Pdf => false,
    };
    p.get("includeBackground").and_then(Value::as_bool).unwrap_or(true) || !keeps_alpha
}

/// Formats whose file names Artboards to Files writes with a `-` for each space, as the reference
/// application does for PNG (and, per #13633, JPEG, BMP, Targa and TIFF). PSD keeps its spaces,
/// as do the formats the reference application was not measured with.
const HYPHENATED_FORMATS: &[&str] = &["png", "jpg", "jpeg", "bmp", "tga", "tif", "tiff"];

/// The file names (without extension) Artboards to Files writes for `names`, in export order:
/// `<prefix><artboard name>` with no separator or index between them, [`sanitize`]d as a whole,
/// and with each space turned into `-` when `hyphenate`.
///
/// A name equal to an earlier one ignoring letter case ([`collision_key`]), or equal once
/// sanitized, gets the first `-2`, `-3`… that no other board's own name and no earlier suffix
/// takes. Every board still gets a file of its own on case-insensitive file systems (#6613), and
/// the first board of a name, like every board whose name is unique, keeps the plain name.
fn artboard_file_stems(prefix: &str, names: &[&str], hyphenate: bool) -> Vec<String> {
    let plain: Vec<String> = names
        .iter()
        .map(|name| {
            let stem = sanitize(&format!("{prefix}{name}"));
            if hyphenate { stem.replace(' ', "-") } else { stem }
        })
        .collect();
    let mut taken: HashSet<String> = plain.iter().map(|stem| collision_key(stem)).collect();
    let mut seen = HashSet::new();
    plain
        .into_iter()
        .map(|stem| {
            if seen.insert(collision_key(&stem)) {
                return stem;
            }
            let mut n = 2u64;
            loop {
                let candidate = format!("{stem}-{n}");
                if taken.insert(collision_key(&candidate)) {
                    break candidate;
                }
                n += 1;
            }
        })
        .collect()
}

/// The key two file names are compared by to decide whether a case-insensitive file system would
/// take them for one file: lower case, then upper case. Upper case alone misses the Kelvin sign
/// `K`, which macOS folds into `K`; lower case alone misses `σ` and `ς`, which NTFS both reads as
/// `Σ`. Merging a few names a file system would keep apart only costs a `-2` suffix.
fn collision_key(stem: &str) -> String {
    stem.to_lowercase().to_uppercase()
}

/// File › Export › Artboards to Files: one `<prefix><artboard name>.<format>` per board, named by
/// [`artboard_file_stems`] (`abfiles-testRed-Board.png`, `abfiles-testRed Board.psd`), or for PDF one
/// `<prefix>.pdf`.
fn artboards_to_files(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.artboardsToFiles";
    let dir = str_param(p, "dir", cmd).or_else(|_| str_param(p, "output", cmd))?.to_string();
    let format = p.get("format").and_then(Value::as_str).unwrap_or("png").trim_start_matches('.').to_ascii_lowercase();
    let (save, board) = export_options::parse(s, p, cmd)?;
    let border = name_border_param(p, cmd)?.filter(|_| !keeps_layers(&format));
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let prefix = p.get("prefix").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&d.doc.name));
    let overlapping = overlapping_param(p, cmd)?;
    let doc = d.doc.clone();
    let guard = ScratchGuard::new(&doc)?;
    let mut boards = chosen_documents(&doc, p, overlapping)?;
    if boards.is_empty() {
        return Err(bad(cmd, "no matching artboards"));
    }
    let file_type = FileType::of(&format, cmd)?;
    // Include Background in Export unticked (#13631): each board's white fill is left out of the
    // files that keep transparency, before the name border extends the canvas.
    if !includes_background(p, &file_type) {
        for one in &mut boards {
            drop_white_fill(one);
        }
    }
    if let Some(o) = &border {
        for one in &mut boards {
            add_name_border(one, o, cmd)?;
        }
    }
    let truecolour = matches!(file_type, FileType::Png24);
    let ext = match file_type {
        FileType::Pdf => {
            // The reference application's Multi-Page Document: the bottom artboard is page 1, and
            // each page is the board's pixel size at 72 ppi.
            let mut pages = Vec::new();
            for one in boards.iter().rev() {
                pages.push((one.size.width, one.size.height, 72.0, jpeg_page(one, save.quality.or(Some(10.0)), cmd)?));
            }
            let name = if prefix.is_empty() { stem(&doc.name) } else { prefix };
            let path = join(&dir, &format!("{}.pdf", sanitize(&name)));
            guard.check()?;
            write_file(&path, &raster_pdf(&pages))?;
            return Ok(json!({"files": [path], "warnings": []}));
        }
        FileType::Png8 => None,
        FileType::Png24 => Some("png".to_string()),
        FileType::Save(ext) => Some(ext),
    };
    let extension = ext.as_deref().unwrap_or("png");
    let names: Vec<&str> = boards.iter().map(|one| one.name.as_str()).collect();
    let stems = artboard_file_stems(&prefix, &names, HYPHENATED_FORMATS.contains(&extension));
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    for (mut one, name) in boards.into_iter().zip(stems) {
        // Export Options of the file type (#13636): PNG-8 and PNG-24 take PNG's.
        board.apply(&mut one, extension);
        let path = join(&dir, &format!("{name}.{extension}"));
        let (bytes, warned) = match &ext {
            Some(_) if truecolour && one.mode != ColorMode::Rgb => encode(&rgb_copy(&one)?, &path, save)?,
            Some(_) => encode(&one, &path, save)?,
            // The PNG-8 writer does not interlace; say so, as the palette PNG path of a save does.
            None if save.png_interlaced == Some(true) => {
                (png8(&one, cmd)?, vec!["written without interlacing: the PNG-8 exporter does not interlace".to_owned()])
            }
            None => (png8(&one, cmd)?, Vec::new()),
        };
        guard.check()?;
        write_file(&path, &bytes)?;
        warnings.extend(warned.into_iter().map(|w| format!("{name}.{extension}: {w}")));
        files.push(path);
    }
    Ok(json!({"files": files, "warnings": warnings}))
}

/// The scratch state an artboard export starts from. Building the boards copies their pixels
/// (and the PNG-8 and PDF paths flatten them) before the encoder's own scratch check sees them,
/// and a tile that fails to read gives placeholder pixels: [`ScratchGuard::check`] refuses them
/// before a file is written, as the export pipeline does for a document it reads itself.
struct ScratchGuard<'a> {
    doc: &'a Document,
    generation: u64,
}

impl<'a> ScratchGuard<'a> {
    fn new(doc: &'a Document) -> Result<Self> {
        photocraft_compose::discard_unreadable_pixel_caches();
        doc.scratch_health().map_err(EngineError::Other)?;
        Ok(Self { doc, generation: photocraft_raster::spill::read_error_generation() })
    }

    /// An error when a scratch read failed since [`ScratchGuard::new`], or the document's own
    /// pixels are unreadable.
    fn check(&self) -> Result<()> {
        photocraft_raster::spill::check_since(self.generation).and_then(|()| self.doc.scratch_health()).map_err(EngineError::Other)
    }
}

/// `one` as an indexed PNG: Generate Assets' PNG-8 (selective palette, no dither, transparency kept).
fn png8(one: &Document, cmd: &str) -> Result<Vec<u8>> {
    use crate::web_cmds::{WebFormat, WebMetadata, WebSettings, optimize, web_document};
    let st = WebSettings {
        format: WebFormat::Png8,
        dither: photocraft_algo::quantize::Dither::None,
        matte: None,
        metadata: WebMetadata::None,
        ..WebSettings::default()
    };
    crate::file_cmds::check_flattenable(one, cmd)?;
    let (wdoc, _, _) = web_document(one, &json!({}), &st)?;
    let buf = photocraft_compose::flatten(&wdoc);
    Ok(optimize(&buf.px, wdoc.size.width as usize, wdoc.bounds(), &st, None, None, None, wdoc.resolution_dpi, false)?.bytes)
}

/// `one` converted to RGB (its depth, layers and transparency kept), for a truecolour file.
fn rgb_copy(one: &Document) -> Result<Document> {
    use crate::web_cmds::{WebSettings, web_document};
    Ok(web_document(one, &json!({}), &WebSettings { convert_to_srgb: false, ..WebSettings::default() })?.0)
}

/// `one` flattened over white as a JPEG, for a PDF page. A board past the compositor's render
/// limit is refused before it is flattened (the flatten would be empty).
fn jpeg_page(one: &Document, quality: Option<f64>, cmd: &str) -> Result<Vec<u8>> {
    Ok(jpeg_page_masked(one, quality, false, cmd)?.0)
}

/// [`jpeg_page`] and, when `masked`, the page's [`soft_mask`] taken from the composite before it
/// goes over white.
fn jpeg_page_masked(one: &Document, quality: Option<f64>, masked: bool, cmd: &str) -> Result<(Vec<u8>, Option<Vec<u8>>)> {
    crate::file_cmds::check_flattenable(one, cmd)?;
    let composite = photocraft_compose::flatten(one);
    let mask = if masked { soft_mask(&composite.px)? } else { None };
    let buf = composite.over_background([1.0, 1.0, 1.0]);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mut flat = Document::new(&one.name, one.size, ColorMode::Rgb, SampleType::U8);
    let mut surf = Surface::new(fmt);
    let vals: Vec<f32> = buf.px.iter().flat_map(|px| photocraft_raster::from_rgba(&fmt, *px)).collect();
    surf.write_region(one.bounds(), &vals);
    flat.layers.push(Layer::new("Background", LayerContent::Raster(surf)));
    Ok((encode(&flat, "page.jpg", quality)?.0, mask))
}

/// A minimal PDF with one page per JPEG image: (width px, height px, dpi, JPEG bytes).
/// Pages are sized so the image prints at its resolution.
pub fn raster_pdf(pages: &[(u32, u32, f32, Vec<u8>)]) -> Vec<u8> {
    let pages: Vec<PdfPage> = pages.iter().map(|(w, h, dpi, jpeg)| (*w, *h, *dpi, jpeg.clone(), None)).collect();
    raster_pdf_masked(&pages)
}

/// One page of [`raster_pdf_masked`]: width px, height px, dpi, JPEG bytes and, when the page
/// has transparency, its alpha as a Flate-compressed 8-bit DeviceGray image of the same size.
type PdfPage = (u32, u32, f32, Vec<u8>, Option<Vec<u8>>);

/// [`raster_pdf`] where a page may carry a soft mask (`/SMask`). The mask has `/Matte [1 1 1]`:
/// the page's colours are already composited over white, so a viewer that honours the mask
/// recovers the true colours, and one that ignores it shows today's white page.
fn raster_pdf_masked(pages: &[PdfPage]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    // Objects: 1 catalog, 2 page tree, then per page: page, contents, image and, when the page
    // has one, its soft mask; numbered in that order.
    let mut next = 3;
    let first: Vec<usize> = pages
        .iter()
        .map(|page| {
            let n = next;
            next += if page.4.is_some() { 4 } else { 3 };
            n
        })
        .collect();
    let kids: Vec<String> = first.iter().map(|n| format!("{n} 0 R")).collect();
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).as_bytes());
    for ((w, h, dpi, jpeg, mask), n) in pages.iter().zip(first) {
        let k = 72.0 / f64::from(dpi.max(1.0));
        let (pw, ph) = (f64::from(*w) * k, f64::from(*h) * k);
        let (contents, image, smask) = (n + 1, n + 2, n + 3);
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.3} {ph:.3}] /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {contents} 0 R >>"
            )
            .as_bytes(),
        );
        let stream = format!("q {pw:.3} 0 0 {ph:.3} 0 0 cm /Im0 Do Q");
        obj(&mut out, format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()).as_bytes());
        let soft = if mask.is_some() { format!(" /SMask {smask} 0 R") } else { String::new() };
        let mut img = format!(
            "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8{soft} /Filter /DCTDecode /Length {} >>\nstream\n",
            jpeg.len()
        )
        .into_bytes();
        img.extend_from_slice(jpeg);
        img.extend_from_slice(b"\nendstream");
        obj(&mut out, &img);
        if let Some(mask) = mask {
            let mut m = format!(
                "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8 /Matte [1 1 1] /Filter /FlateDecode /Length {} >>\nstream\n",
                mask.len()
            )
            .into_bytes();
            m.extend_from_slice(mask);
            m.extend_from_slice(b"\nendstream");
            obj(&mut out, &m);
        }
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

/// Leaves out `one`'s board fill for Artboards to PDF's Include Background in Export unticked
/// (#13648), so what the board's layers don't cover becomes transparent. Only what the reference
/// application was measured with: a white board with layers in an 8-bit RGB document. Anything
/// else (an empty board, another fill, other modes and depths) keeps today's opaque page; the
/// return value says whether the fill was left out.
fn drop_white_fill(one: &mut Document) -> bool {
    if one.mode != ColorMode::Rgb || one.depth != SampleType::U8 {
        return false;
    }
    let mut dropped = false;
    for board in &mut one.layers {
        if board.children().is_some_and(|layers| !layers.is_empty())
            && let Some(artboard) = board.artboard_mut()
            && artboard.background == ArtboardBackground::White
        {
            artboard.background = ArtboardBackground::Transparent;
            dropped = true;
        }
    }
    dropped
}

/// A page's soft mask: its alpha as 8-bit gray, Flate-compressed. `None` when the page is opaque
/// everywhere, so a board its layers cover entirely keeps today's page.
fn soft_mask(px: &[[f32; 4]]) -> Result<Option<Vec<u8>>> {
    let alpha: Vec<u8> = px.iter().map(|p| (p[3].clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    if alpha.iter().all(|a| *a == 255) {
        return Ok(None);
    }
    let failed = |e: std::io::Error| EngineError::Other(format!("Artboards to PDF: soft mask: {e}"));
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    std::io::Write::write_all(&mut z, &alpha).map_err(failed)?;
    Ok(Some(z.finish().map_err(failed)?))
}

/// File › Export › Artboards to PDF: one page per board (flattened over white, JPEG-encoded),
/// all in one file at `path`, or with `"mode": "perArtboard"` one single-page file per board
/// named by [`pdf_file_stems`]. Every page is encoded before any file is written.
/// With `"includeBackground": false` a white board's fill is left out and the page gets a soft
/// mask, so a viewer shows its own background where the board has no pixels.
fn artboards_to_pdf(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.artboardsToPdf";
    let per_artboard = match p.get("mode") {
        None | Some(Value::Null) => false,
        Some(Value::String(m)) if m == "multiPage" => false,
        Some(Value::String(m)) if m == "perArtboard" => true,
        Some(other) => return Err(bad(cmd, format!("\"mode\" must be \"multiPage\" or \"perArtboard\", not {other}"))),
    };
    let path = if per_artboard { p.get("path").and_then(Value::as_str).unwrap_or_default().to_string() } else { str_param(p, "path", cmd)?.to_string() };
    // Document Per Artboard: the folder and prefix default to those of `path` (the dialog's
    // `<folder>/<document name>.pdf`), so `<folder>/<document name><artboard name>.pdf`.
    let dir = match p.get("dir").and_then(Value::as_str).filter(|d| !d.is_empty()) {
        Some(d) => d.to_string(),
        None if per_artboard && path.is_empty() => return Err(bad(cmd, "missing \"dir\" or \"path\"")),
        None => folder_of(&path),
    };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    let prefix = match p.get("prefix").and_then(Value::as_str) {
        Some(prefix) => prefix.to_string(),
        None if path.is_empty() => stem(&doc.name),
        None => stem(&path),
    };
    let quality = f64_param(p, "quality").or(Some(10.0));
    let background = p.get("includeBackground").and_then(Value::as_bool).unwrap_or(true);
    // Reverse Page Order (#13644): the default page order the other way round.
    let reverse = match p.get("reversePageOrder") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "`reversePageOrder` must be true or false"))?,
    };
    let overlapping = overlapping_param(p, cmd)?;
    let guard = ScratchGuard::new(&doc)?;
    let mut pages = Vec::new();
    let mut names = Vec::new();
    // Pages go bottom artboard of the Layers panel first, as in the reference application (#13643);
    // `chosen_board_documents` lists them top first, the order Artboards to Files keeps.
    for (id, mut one) in chosen_board_documents(&doc, p, overlapping)?.into_iter().rev() {
        // A hidden board was not measured and keeps today's opaque page. `artboard_document`
        // shows the board, so its visibility is read from the source.
        let visible = doc.layer(id).is_some_and(|l| l.visible);
        let masked = !background && visible && drop_white_fill(&mut one);
        let (jpeg, mask) = jpeg_page_masked(&one, quality, masked, cmd)?;
        pages.push((one.size.width, one.size.height, doc.resolution_dpi, jpeg, mask));
        names.push(one.name);
    }
    if pages.is_empty() {
        return Err(bad(cmd, "no matching artboards"));
    }
    if reverse {
        pages.reverse();
        names.reverse();
    }
    guard.check()?;
    if !per_artboard {
        write_file(&path, &raster_pdf_masked(&pages))?;
        return Ok(json!({"path": path, "pages": pages.len(), "files": [path]}));
    }
    let mut files = Vec::new();
    for (page, name) in pages.iter().zip(pdf_file_stems(&prefix, &names)) {
        let file = join(&dir, &format!("{name}.pdf"));
        write_file(&file, &raster_pdf_masked(std::slice::from_ref(page)))?;
        files.push(file);
    }
    Ok(json!({"pages": pages.len(), "files": files}))
}

/// The folder part of `path`: `""` for a bare file name, and the root itself for a file at
/// the root (`/x.pdf` gives `/`, not the working directory).
fn folder_of(path: &str) -> String {
    path.rfind(['/', '\\']).map_or(String::new(), |i| path[..i.max(1)].to_string())
}

/// Document Per Artboard's file names: `<prefix><artboard name>`, spaces kept and no separator
/// added, with characters a file name can't hold as `_`. A name an earlier board already took,
/// ignoring letter case, gets the first `-2`, `-3`… no other board's name takes, so every board
/// keeps a file of its own on case-insensitive file systems; the first board of a name keeps it.
fn pdf_file_stems(prefix: &str, names: &[String]) -> Vec<String> {
    // Upper case of lower case also folds `ß`/`ẞ` and `σ`/`ς`/`Σ` together, as NTFS does.
    let key = |stem: &str| stem.to_lowercase().to_uppercase();
    let plain: Vec<String> = names.iter().map(|name| sanitize(&format!("{prefix}{name}"))).collect();
    let mut taken: HashSet<String> = plain.iter().map(|stem| key(stem)).collect();
    let mut seen = HashSet::new();
    plain
        .into_iter()
        .map(|stem| {
            if seen.insert(key(&stem)) {
                return stem;
            }
            // At most 2 x names keys are taken, so one of these many suffixes is always free.
            let last = u64::try_from(names.len()).unwrap_or(u64::MAX).saturating_mul(2).saturating_add(2);
            (2..=last).map(|n| format!("{stem}-{n}")).find(|candidate| taken.insert(key(candidate))).unwrap_or(stem)
        })
        .collect()
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!(
            "layer.new.artboard",
            "Artboard…",
            &["Layer", "New"],
            r##"{"rect":[x,y,w,h]? | "x","y","width","height"? (default: canvas size, right of the last board),"preset":"iPhone 14|Web 1920|A4|…"?,"name":str?,"background":"white|black|transparent|custom"="white","color":[r,g,b]|"#rrggbb"? (custom),"takeBackground":bool=false (the first board in the document also takes in the Background as a normal layer "Layer 0"; the Artboard tool's drag sets it)} → {layer, rect}"##,
            has_doc,
            new_artboard
        ),
        spec!(
            "layer.new.artboardFromGroup",
            "Artboard from Group",
            &["Layer", "New"],
            r##"{"layer":id? (a top-level group; default active),"background":…?,"color":[r,g,b]|"#rrggbb"?,"preset":str? (tag only; does not resize),"guideIDs":[int]? (ignored recording metadata),"changeBackground":int? (1 or absent applies background/color; other integers give white; legacy true = 1),"recordedColor"? (ATN round-trip of the recorded color),"backgroundTypeOmitted":bool? (ATN: the descriptor omitted the background type)} → {layer, rect}"##,
            has_plain_group,
            artboard_from_group
        ),
        spec!(
            "layer.new.artboardFromLayers",
            "Artboard from Layers…",
            &["Layer", "New"],
            r##"{"name":text,"background":"white|black|transparent|custom"="white","color":color,"width":px=0,"height":px=0} (the selected layers; name: empty gives Artboard N; color: for custom; width, height: the board's size from the layers' top left, 0 keeps their bounds) → {layer, rect}"##,
            has_free_layers,
            artboard_from_layers
        ),
        spec!(
            "layer.artboard.set",
            "Edit Artboard",
            &[],
            r##"{"layer":id? (default: the active artboard),"x","y","width","height"?|"rect":[x,y,w,h]?,"preset":str?,"background":"white|black|transparent|custom"?,"color":…?,"name":str?,"moveContents":bool=true} → {layer, rect}"##,
            has_active_artboard,
            set_props
        ),
        spec!(
            "view.clearSelectedArtboardGuides",
            "Clear Selected Artboard Guides",
            &["View"],
            "{} (guides inside the active artboard)",
            has_active_artboard,
            |s, _| clear_artboard_guides(s)
        ),
        spec!(
            "layer.artboard.addBeside",
            "Add Artboard Beside",
            &[],
            r##"{"layer":id? (default: the active artboard),"side":"left|right|top|bottom"="right","duplicate":bool=false (copy the board with its contents instead of an empty one)} → {layer, rect} (the Artboard tool's "+" buttons: same size, a 100 px gap away, past any board in the way)"##,
            has_active_artboard,
            add_beside
        ),
        spec!(
            "file.export.artboardsToFiles",
            "Artboards to Files…",
            &["File", "Export"],
            r##"{"dir":folder,"format":"png|png24|png8|jpg|bmp|tga|tiff|psd|pdf|…"="png" (png24 = truecolour .png, png8 = indexed .png, tga = Targa; pdf = one multi-page <prefix>.pdf, bottom artboard first, pages at 72 ppi),"prefix":str=document name ("" = none),"content":"overlapping|artboard"="overlapping" (Include Overlapping Areas: a board's file also shows the visible boards above it where they overlap it, each as a plain group with a \"Background for <board>\" layer; artboard: Artboard Content Only),"artboards":[id]? (default all),"quality":0..12? (JPEG, and the PDF pages),"iccProfile":bool=true,"bmpBits":24|32? (16 unsupported),"tgaBits":24|32?,"tiffCompression":"none|lzw|zip"="none" ("jpeg" unsupported),"pngInterlaced":bool=false (PNG, PNG-24),"transparency":bool=true (PNG, PNG-8, PNG-24; false: over white, no alpha),"includeBackground":bool=true (false: each board's white fill is left out of PNG, PNG-24, TIFF and WebP files, so they come out transparent where the board's layers have no pixels; an empty board, other fills, documents other than 8-bit RGB, PSD, PNG-8, PDF and formats without alpha keep it),"maximizeCompatibility":bool? (PSD),"preserveArtboard":bool=true (PSD; false: a plain group),"includeName":bool=false (Include Artboard Name: a border of (w + h) / 20 px on each side, the name in the top one; psd and pdf keep their layers),"font":family="Arial","fontStyle":"Regular|Italic|Bold|Bold Italic|Black"="Regular","fontSize":pt|numeric str=12 (else an error),"extension":"white|black|other"|"#rrggbb"|[r,g,b]="white" (Canvas Extension; "other" takes extensionColor),"extensionColor":"#rrggbb"|[r,g,b]? (used when extension is "other"),"textColor":"#rrggbb"|[r,g,b]=black} → {files, warnings} (warnings: what a file could not hold, each "<file name>: <warning>"; one "<prefix><artboard name>.<extension>" per board, no separator or index added: spaces become "-" in PNG, JPEG, BMP, Targa and TIFF and stay in PSD and other formats, characters a file name can't hold become "_", and a name an earlier board already took, ignoring case, gets "-2", "-3"…)"##,
            export_artboards,
            artboards_to_files
        ),
        spec!(
            "file.export.artboardsToPdf",
            "Artboards to PDF…",
            &["File", "Export"],
            r##"{"path":str (.pdf; with "mode":"perArtboard" only gives the defaults of "dir" and "prefix"),"mode":"multiPage|perArtboard"="multiPage" (Multi-Page Document: one file at "path"; Document Per Artboard: one single-page file per board, "<dir>/<prefix><artboard name>.pdf", spaces kept, characters a file name can't hold become "_", a name an earlier board already took, ignoring case, gets "-2", "-3"…),"dir":folder=folder of "path","prefix":str=file name of "path" without ".pdf", else the document name,"content":"overlapping|artboard"="overlapping" (Include Overlapping Areas: a board's page also shows the visible boards above it where they overlap it; artboard: Artboard Content Only),"artboards":[id]?,"quality":0..12=10,"includeBackground":bool=true (false: a white board's fill is left out and the page gets a soft mask, so it is transparent where the board's layers have no pixels; an empty board, other fills and documents other than 8-bit RGB keep it),"reversePageOrder":bool=false (the default page order the other way round)} → {path (multi-page), pages, files} (one raster page per board, bottom artboard of the Layers panel first)"##,
            export_artboards,
            artboards_to_pdf
        ),
    ]
}

#[path = "artboard_cmds/export_options.rs"]
mod export_options;

#[cfg(test)]
#[path = "artboard_cmds/export_options_tests.rs"]
mod export_options_tests;
#[cfg(test)]
#[path = "artboard_cmds/file_names_tests.rs"]
mod file_names_tests;
#[cfg(test)]
#[path = "artboard_cmds/file_types_tests.rs"]
mod file_types_tests;
#[cfg(test)]
#[path = "artboard_cmds/first_board_background_tests.rs"]
mod first_board_background_tests;
#[cfg(test)]
#[path = "artboard_cmds/from_group_reset_props_tests.rs"]
mod from_group_reset_props_tests;
#[cfg(test)]
#[path = "artboard_cmds/include_background_tests.rs"]
mod include_background_tests;
#[cfg(test)]
#[path = "artboard_cmds/include_name_tests.rs"]
mod include_name_tests;

#[cfg(test)]
#[path = "artboard_cmds/from_layers_history_name_tests.rs"]
mod from_layers_history_name_tests;

#[cfg(test)]
#[path = "artboard_cmds/from_group_hidden_bounds_tests.rs"]
mod from_group_hidden_bounds_tests;

#[cfg(test)]
#[path = "artboard_cmds/overlapping_areas_tests.rs"]
mod overlapping_areas_tests;
#[cfg(test)]
#[path = "artboard_cmds/pdf_document_per_artboard_tests.rs"]
mod pdf_document_per_artboard_tests;

#[cfg(test)]
#[path = "artboard_cmds/pdf_overlapping_areas_tests.rs"]
mod pdf_overlapping_areas_tests;
#[cfg(test)]
#[path = "artboard_cmds/pdf_page_order_tests.rs"]
mod pdf_page_order_tests;
#[cfg(test)]
#[path = "artboard_cmds/pdf_reverse_page_order_tests.rs"]
mod pdf_reverse_page_order_tests;
#[cfg(test)]
#[path = "artboard_cmds/tests.rs"]
mod tests;
