use craft::behavior::kurbo;
use photocraft_text::warp::Warp;

use super::{Affine, Arc, Document, LayerContent, LayerId, PhotocraftApp, Point, PoisonError, TextLayer, TextLayout, text_layer};

/// Layout of a type layer (cached per document revision) and its text → document transform.
pub fn layout(app: &mut PhotocraftApp, id: LayerId) -> Option<(Arc<TextLayout>, Affine, String)> {
    let dragged = crate::type_transform::current_transform(app, id);
    // Borrows the document in place (no clone): the canvas hover hit-tests type every frame.
    let st = app.session.active()?;
    let t = text_layer(&st.doc, id)?;
    let transform = dragged.unwrap_or(t.transform);
    let key = (st.doc.id.0, st.revision, id.0);
    if let Some((k, l)) = &app.type_layout
        && *k == key
    {
        return Some((l.clone(), transform, t.text.clone()));
    }
    let l = Arc::new(photocraft_text::shared().lock().ok()?.layout(t, st.doc.resolution_dpi));
    app.type_layout = Some((key, l.clone()));
    Some((l, transform, t.text.clone()))
}

pub(super) fn to_text(aff: &Affine, x: f64, y: f64) -> (f32, f32) {
    let p = aff.inverse().unwrap_or(Affine::IDENTITY).apply(Point::new(x, y));
    (p.x as f32, p.y as f32)
}

/// Topmost shown type layer (it and every group around it visible, #4320) whose laid-out text
/// contains the document point. The press and the canvas hover (`hover_path_target`) share it, so
/// the pointer never promises a path click that the press would turn into editing this layer.
pub(crate) fn hit_layer(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<LayerId> {
    let slop = hit_slop(app);
    hit_candidates(app)?.into_iter().find(|id| {
        // The pixels shown count too: a PSD's type keeps Photoshop's rendering until edited,
        // which can sit off our layout when fonts are substituted (#123).
        let shown = shown_bounds(app, *id, slop).is_some_and(|r| x >= r.x0 && x <= r.x1 && y >= r.y0 && y <= r.y1);
        // The hover calls this every frame: skip the layout when the pixels already answer.
        if shown {
            return true;
        }
        let Some((l, aff, _)) = layout(app, *id) else { return false };
        let (tx, ty) = to_text(&aff, x, y);
        photocraft_text::text_point_inside(&l, tx, ty, slop)
    })
}

/// How far from a type layer's text a press still lands on it, in document units: 6 screen points.
fn hit_slop(app: &PhotocraftApp) -> f32 {
    6.0 / app.point_zoom().max(0.01)
}

/// The type layers a press can land on, topmost first: the shown ones, and a Type Mask session's text, a hidden layer
/// that is still the text being edited.
fn hit_candidates(app: &PhotocraftApp) -> Option<Vec<LayerId>> {
    let editing = app.ui.text_edit.as_ref().filter(|ed| ed.mask).map(|ed| LayerId(ed.layer));
    let doc = &app.session.active()?.doc;
    let mut ids: Vec<LayerId> = doc
        .walk()
        .into_iter()
        .filter(|(path, _, l)| matches!(l.content, LayerContent::Text(_)) && (editing == Some(l.id) || doc.shown_at(path)))
        .map(|(_, _, l)| l.id)
        .collect();
    ids.reverse(); // walk() is bottom-up; hit the topmost first
    Some(ids)
}

/// The bounds of the pixels `id` shows, grown by `slop`, in document units; `None` with nothing shown.
fn shown_bounds(app: &PhotocraftApp, id: LayerId, slop: f32) -> Option<kurbo::Rect> {
    let r = text_layer(&app.session.active()?.doc, id)?.cache.as_ref()?.content_bounds();
    let s = f64::from(slop);
    (!r.is_empty()).then(|| kurbo::Rect::new(f64::from(r.x0) - s, f64::from(r.y0) - s, f64::from(r.x1) + s, f64::from(r.y1) + s))
}

/// The two areas [`hit_layer`] finds `id` in, as document quads: its shown pixels' bounds and its layout's bounds
/// through its transform (`text_point_inside`), each grown by `slop`.
fn hit_quads(app: &mut PhotocraftApp, id: LayerId, slop: f32) -> Vec<[kurbo::Point; 4]> {
    let mut quads = Vec::new();
    if let Some(r) = shown_bounds(app, id, slop) {
        quads.push([kurbo::Point::new(r.x0, r.y0), kurbo::Point::new(r.x1, r.y0), kurbo::Point::new(r.x1, r.y1), kurbo::Point::new(r.x0, r.y1)]);
    }
    let laid = layout(app, id).and_then(|(l, aff, _)| Some((l.bounds()?, aff)));
    if let Some((b, aff)) = laid {
        let s = f64::from(slop);
        let text = kurbo::Rect::new(f64::from(b[0]) - s, f64::from(b[1]) - s, f64::from(b[2]) + s, f64::from(b[3]) + s);
        quads.extend(crate::type_transform::quad_of(text, &aff));
    }
    quads
}

/// The areas where a plain press, with an edit open, is the Type tool's press that commits the edit rather than the
/// edit's (its move reach, its caret, its box), in document units. Production's order of checks for that press
/// (pc-prod-e76 tools/type_tool/pointer.rs:117-150, tools/type_transform.rs:185-196, 303-330): a handle of the
/// edited paragraph box first (`box_handle_at`, pointer.rs:31-52), then a type layer other than the edited one that
/// [`hit_layer`] finds topmost there (it commits, and is never the move reach), then the edited text or the inside of
/// its box (the caret). So the areas are each other layer's hit areas, less the edited layer's where it lies above
/// that layer, less the edited box and its handle squares. They are kept to the edited frame grown by the move reach:
/// further out nothing of the edit takes a press, which is the tool's anyway.
pub(crate) fn claimed_regions(app: &mut PhotocraftApp) -> Vec<Vec<kurbo::Point>> {
    let Some(ed) = app.ui.text_edit.clone() else { return Vec::new() };
    if super::editing::edit_home(app, &ed) != app.session.active_index() {
        return Vec::new();
    }
    let id = LayerId(ed.layer);
    let Some((frame, aff)) = crate::type_transform::source_rect(app, id) else { return Vec::new() };
    let Some(edited_frame) = crate::type_transform::quad_of(frame, &aff) else { return Vec::new() };
    let zoom = f64::from(app.point_zoom().max(0.01));
    let reach_out = crate::type_transform::MOVE_REACH_PT / zoom;
    let slop = hit_slop(app);
    let Some(ids) = hit_candidates(app) else { return Vec::new() };
    // Where the edited layer stacks among them; a hidden one is no hit.
    let edited_at = ids.iter().position(|&other| other == id);
    // The edited box and its handle squares (pointer.rs:31-52: 6 points around each corner and side middle, the
    // enlarged corner of an overflowing box half a diagonal of its 11-point square).
    let mut cuts: Vec<[kurbo::Point; 4]> = Vec::new();
    if super::pointer::box_shape(app, id).is_some() {
        cuts.push(edited_frame);
        let tol = 6.0 / zoom;
        cuts.extend(edited_frame.iter().map(|&corner| square(corner, tol)));
        cuts.extend((0..4).map(|k| square(edited_frame[k].midpoint(edited_frame[(k + 1) % 4]), tol)));
        if app.ui.type_box_overflow {
            cuts.push(square(edited_frame[2], 11.0 / 2.0 * std::f64::consts::SQRT_2 / zoom));
        }
    }
    let edited = if edited_at.is_some() { hit_quads(app, id, slop) } else { Vec::new() };
    let mut regions = Vec::new();
    for (at, &other) in ids.iter().enumerate() {
        if other == id {
            continue;
        }
        let mut pieces: Vec<Vec<kurbo::Point>> = hit_quads(app, other, slop).into_iter().map(|q| clip_outset(&q, &edited_frame, reach_out)).collect();
        let below_edited = edited_at.is_some_and(|edited_at| edited_at < at);
        for cut in edited.iter().filter(|_| below_edited).chain(&cuts) {
            pieces = pieces.iter().flat_map(|piece| minus(piece, cut)).collect();
        }
        regions.extend(pieces.into_iter().filter(|piece| piece.len() >= 3));
    }
    regions
}

/// The document square of half side `half` around `at`.
fn square(at: kurbo::Point, half: f64) -> [kurbo::Point; 4] {
    [(-half, -half), (half, -half), (half, half), (-half, half)].map(|(dx, dy)| kurbo::Point::new(at.x + dx, at.y + dy))
}

/// How far `p` lies inside side `k` of the convex polygon `poly`, scaled by the side's length: positive inside.
fn inside_side(poly: &[kurbo::Point], k: usize, p: kurbo::Point) -> f64 {
    let area: f64 = (0..poly.len()).map(|i| poly[i].to_vec2().cross(poly[(i + 1) % poly.len()].to_vec2())).sum();
    let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
    (b - a).cross(p - a) * area.signum()
}

/// The part of the convex `piece` where `keep` is at least zero (one side of a line, as a linear function of the
/// point).
fn clip(piece: &[kurbo::Point], keep: impl Fn(kurbo::Point) -> f64) -> Vec<kurbo::Point> {
    let mut out = Vec::new();
    for (i, &here) in piece.iter().enumerate() {
        let before = piece[(i + piece.len() - 1) % piece.len()];
        let (a, b) = (keep(before), keep(here));
        if (a >= 0.0) != (b >= 0.0) {
            out.push(before + (here - before) * (a / (a - b)));
        }
        if b >= 0.0 {
            out.push(here);
        }
    }
    out
}

/// The convex `piece` within `distance` outside every side of the convex quad `quad`.
fn clip_outset(piece: &[kurbo::Point], quad: &[kurbo::Point; 4], distance: f64) -> Vec<kurbo::Point> {
    (0..4).fold(piece.to_vec(), |piece, k| {
        let length = (quad[(k + 1) % 4] - quad[k]).hypot();
        if piece.len() < 3 || length <= 0.0 {
            return piece;
        }
        clip(&piece, |p| inside_side(quad, k, p) + distance * length)
    })
}

/// The convex `piece` less the convex quad `cut`, as convex pieces: for each side of `cut`, the part of what is left
/// that lies outside that side.
fn minus(piece: &[kurbo::Point], cut: &[kurbo::Point; 4]) -> Vec<Vec<kurbo::Point>> {
    let area: f64 = (0..4).map(|i| cut[i].to_vec2().cross(cut[(i + 1) % 4].to_vec2())).sum();
    if piece.len() < 3 || area.abs() < 1e-12 {
        return vec![piece.to_vec()];
    }
    let mut rest = piece.to_vec();
    let mut out = Vec::new();
    for k in 0..4 {
        let outside = clip(&rest, |p| -inside_side(cut, k, p));
        if outside.len() >= 3 {
            out.push(outside);
        }
        rest = clip(&rest, |p| inside_side(cut, k, p));
        if rest.len() < 3 {
            break;
        }
    }
    out
}

/// True when the layer's pixels are what our engine draws for it. A PSD's type layer keeps
/// Photoshop's pixels until it is edited, and with substituted fonts or a different line layout
/// they sit somewhere else than the glyphs the caret and selection are placed on.
pub(crate) fn shows_own_layout(doc: &Document, t: &TextLayer) -> bool {
    let Some(cache) = &t.cache else { return false };
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(PoisonError::into_inner);
    let ours = eng.render(t, doc.resolution_dpi, doc.pixel_format()).1.surface.content_bounds();
    let have = cache.content_bounds();
    [(ours.x0, have.x0), (ours.y0, have.y0), (ours.x1, have.x1), (ours.y1, have.y1)].iter().all(|(a, b)| (a - b).abs() <= 1)
}

/// Whether layer `id` runs its text vertically.
pub(super) fn is_vertical(app: &PhotocraftApp, id: LayerId) -> bool {
    app.session.active().and_then(|s| text_layer(&s.doc, id)).is_some_and(|t| t.orientation == photocraft_doc::text::Orientation::Vertical)
}

/// The warp of a layer set on a path (type on a path), `None` for any other text. The path carries the line, so
/// carets, selections and hits go through it.
pub(super) fn path_warp(app: &PhotocraftApp, id: LayerId, l: &TextLayout) -> Option<Warp> {
    let t = text_layer(&app.session.active()?.doc, id)?;
    let text_start = l.line_bounds().map_or(0.0, |bounds| f64::from(bounds[0]));
    Warp::on_path_oriented(&t.shape, text_start, l.vertical)
}

/// A line-space point (along the line, across it) in document space: the line turns to text space, bends along the
/// path when there is one, and the layer's transform places it.
pub(super) fn doc_point(l: &TextLayout, aff: &Affine, warp: Option<&Warp>, x: f32, y: f32) -> Point {
    let (tx, ty) = l.to_text(x, y);
    let (tx, ty) = match warp {
        Some(w) => {
            let (wx, wy) = w.apply(f64::from(tx), f64::from(ty));
            (wx as f32, wy as f32)
        }
        None => (tx, ty),
    };
    aff.apply(Point::new(f64::from(tx), f64::from(ty)))
}

/// The character offset of the caret position nearest document point (`x`, `y`) in layer `id`, and the laid-out
/// line the point is on.
pub(super) fn hit_at(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> (usize, usize) {
    let Some((l, aff, text)) = layout(app, id) else { return (0, 0) };
    let (tx, ty) = to_text(&aff, x, y);
    // Type on a path: the point's place along the path is the position in the (single) line.
    let text_end = l.line_bounds().map_or(0.0, |bounds| f64::from(bounds[2]));
    let (tx, ty) = path_warp(app, id, &l)
        .and_then(|warp| warp.path_text_x_near_text(f64::from(tx), f64::from(ty), text_end))
        .map_or((tx, ty), |offset| l.to_text(offset as f32, -1.0));
    photocraft_text::hit_char(&l, &text, tx, ty)
}

/// The character offset of the caret position nearest document point (`x`, `y`) in layer `id`.
pub(crate) fn offset_at(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> usize {
    hit_at(app, id, x, y).0
}
