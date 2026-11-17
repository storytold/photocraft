//! Ctrl/Cmd while typing: an oriented frame and a temporary affine drag, without ending typing.
//! The framework's transform session and overlay do the box; this module holds Type's rules for them (no
//! distort, legacy scaling, the move zone just outside the text) and hands the result to `type.edit` on
//! release.

use std::collections::VecDeque;
use std::sync::Arc;

use craft::behavior::geometry::quad::{BoxAllow, BoxState};
use craft::behavior::kurbo;
use craft::behavior::{CanvasView, GesturePhase};
use craft::draw::ui::transform::{RotateZone, TransformEdit, TransformOverlay, TransformReach, TransformSession, TransformSettings};
use craft::keymap::Mods;
use craft::ui::canvas::canvas_space;
use craft::ui::runtime::env::{Env, Label};
use photocraft_doc::{DocId, Document, LayerContent, LayerId, text::TextShape};
use photocraft_geom::{Affine, Point, Rect};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::TypeTransform;

const PREVIEW_BASE: u64 = 1 << 37;
/// How far outside the text being typed a plain press moves it, as the Move tool (#5956), in
/// screen points. Photoshop 2025, measured at 100 %: a drag from 35 pt off the glyphs moved the
/// text without ending typing, one from 48 pt committed it instead. Kept under 40 pt, so a press
/// 40 pt outside a paragraph box still commits the text (#7606).
pub(crate) const MOVE_REACH_PT: f64 = 38.0;
/// How far outside the text's sides a press with ⌘ held rotates it, in screen points; further out the press does
/// nothing (production `ROTATE_PX`, type_transform.rs:213-216, 310-318).
const ROTATE_REACH_PT: f32 = 28.0;

/// Cached rendered preview and bounded damage history. Unchanged pointer positions render nothing.
pub(crate) struct Preview {
    document: DocId,
    revision: u64,
    transform: Affine,
    shown: Option<Arc<Document>>,
    key: u64,
    original_area: Rect,
    areas: VecDeque<(u64, Rect)>,
}

/// The text's frame: the source rect in text space and the box it stands in on the canvas.
pub(crate) struct Frame {
    /// The edited layer.
    pub layer: LayerId,
    /// The frame in text space.
    pub rect: kurbo::Rect,
    /// The frame's corners through the layer's transform, and the reference point.
    pub state: BoxState,
}

pub(crate) fn active(app: &PhotocraftApp) -> bool {
    app.ui.type_transform.is_some()
}

/// Whether the transform box shows: while typing with ⌘ held (the held modifiers include the ones the app latches),
/// or while a capture of it runs.
pub(crate) fn visible(app: &PhotocraftApp, mods: Mods) -> bool {
    app.ui.tool.is_type() && app.ui.text_edit.is_some() && (mods.cmd || active(app))
}

/// Whether the whole box takes the pointer, above the editor, rather than only its move reach, below it. A capture
/// keeps the box it began on to its release, whatever modifiers change during it; otherwise ⌘ held picks the whole
/// box.
pub(crate) fn whole_box(app: &PhotocraftApp, mods: Mods) -> bool {
    match &app.ui.type_transform {
        Some(t) => !t.from_reach,
        None => mods.cmd,
    }
}

/// The canvas view of the pass the control is shown in; `None` outside a canvas.
pub(crate) fn view_of(ui: &egui::Ui) -> Option<CanvasView> {
    canvas_space(&Env::of(ui).view()).map(|space| space.view)
}

/// The Type tool's rules for Free Transform: corners stretch freely and ⇧ keeps the proportions (the legacy rule).
fn settings() -> TransformSettings {
    let mut settings = TransformSettings::default();
    settings.legacy = true;
    settings
}

/// A session over `frame` with what type allows: moving, scaling, rotating and skewing, never distorting or warping.
fn session_of(app: &PhotocraftApp, frame: &Frame) -> TransformSession<()> {
    TransformSession::<()>::with_state(frame.rect, frame.state)
        .allow(BoxAllow::new(true, true, false).with_warp(false))
        .settings(&settings(), crate::transform_tool::default_interpolation(app))
}

/// Drop pointer capture when its source changed externally. Never apply to another document.
pub(crate) fn cancel_stale(app: &mut PhotocraftApp) {
    let Some(t) = &app.ui.type_transform else { return };
    let valid = app.ui.tool.is_type()
        && app.ui.text_edit.as_ref().is_some_and(|ed| {
            app.session.active().is_some_and(|st| {
                st.doc.id.0 == t.document
                    && st.revision == t.revision
                    && st.doc.layer(LayerId(ed.layer)).is_some_and(|l| !l.locks.all && !l.locks.position && matches!(l.content, LayerContent::Text(_)))
            })
        });
    if !valid {
        let lost_layer = app
            .ui
            .text_edit
            .as_ref()
            .is_none_or(|ed| app.session.active().is_none_or(|st| st.doc.id.0 != t.document || st.doc.layer(LayerId(ed.layer)).is_none()));
        cancel_drag(app);
        if lost_layer {
            app.ui.text_edit = None;
            app.ui.text_edit_return_tool = None;
        }
    }
}

/// Ends the transform without applying it. A running box gesture ends with it: the box is not shown next pass.
pub(crate) fn cancel_drag(app: &mut PhotocraftApp) {
    app.ui.type_transform = None;
    if let Some(p) = app.type_transform_preview.as_mut() {
        // A closed last document has no next canvas frame to release its preview's tiles.
        p.shown = None;
    }
}

pub(crate) fn reset(app: &mut PhotocraftApp) {
    cancel_drag(app);
    app.ui.type_transform_pivot = None;
}

/// The frame of type layer `id` in text space (its paragraph box, the area's bounds, or the laid-out text) and the
/// layer's transform to the document. `None` for a layer that is not type or whose geometry is not finite.
pub(crate) fn source_rect(app: &mut PhotocraftApp, id: LayerId) -> Option<(kurbo::Rect, Affine)> {
    let st = app.session.active()?;
    let LayerContent::Text(text) = &st.doc.layer(id)?.content else { return None };
    let shape = text.shape.clone();
    let size = text.size_pt;
    let (layout, aff, _) = crate::type_tool::layout(app, id)?;
    if !aff.m.iter().all(|v| v.is_finite()) || aff.inverse().is_none() {
        return None;
    }
    let area = match &shape {
        TextShape::Area { path } => photocraft_text::area::path_bounds(path),
        _ => None,
    };
    let rect = match (shape, area) {
        (TextShape::Box { x, y, width, height }, _) => [x, y, x + width, y + height].map(f64::from),
        (_, Some((x0, y0, x1, y1))) => [x0, y0, x1, y1],
        _ => {
            let mut r = layout.bounds().unwrap_or([0.0, -size * layout.px_per_pt, 1.0, 0.0]).map(f64::from);
            // Empty lines still have a caret and can be moved/scaled, without inventing glyphs.
            r[2] = r[2].max(r[0] + 1.0);
            r[3] = r[3].max(r[1] + 1.0);
            r
        }
    };
    if !rect.iter().all(|v| v.is_finite()) || rect[2] <= rect[0] || rect[3] <= rect[1] {
        return None;
    }
    Some((kurbo::Rect::new(rect[0], rect[1], rect[2], rect[3]), aff))
}

/// The frame's corners through `aff`, clockwise from the top left.
pub(crate) fn quad_of(rect: kurbo::Rect, aff: &Affine) -> Option<[kurbo::Point; 4]> {
    let quad = [(rect.x0, rect.y0), (rect.x1, rect.y0), (rect.x1, rect.y1), (rect.x0, rect.y1)].map(|(x, y)| {
        let p = aff.apply(Point::new(x, y));
        kurbo::Point::new(p.x, p.y)
    });
    quad.iter().all(|p| p.x.is_finite() && p.y.is_finite()).then_some(quad)
}

/// The text's frame: the open transform's, else the layer's bounds mapped by its existing affine (including
/// rotation and skew). `None` for a locked, hidden or position-locked layer, which cannot be moved.
pub(crate) fn frame(app: &mut PhotocraftApp) -> Option<Frame> {
    let id = LayerId(app.ui.text_edit.as_ref()?.layer);
    if let Some(t) = &app.ui.type_transform {
        return Some(Frame { layer: id, rect: t.session.rect(), state: *t.session.state() });
    }
    let layer = app.session.active()?.doc.layer(id)?;
    if layer.locks.all || layer.locks.position || !layer.visible {
        return None;
    }
    let (rect, aff) = source_rect(app, id)?;
    let quad = quad_of(rect, &aff)?;
    let pivot = match app.ui.type_transform_pivot {
        Some([x, y]) => kurbo::Point::new(x, y),
        None => quad[0].midpoint(quad[2]),
    };
    Some(Frame { layer: id, rect, state: BoxState::new(quad, pivot) })
}

/// Starts a temporary transform of `frame`, begun on the move reach of the hidden box when `from_reach`. False when it
/// can't start.
fn start(app: &mut PhotocraftApp, frame: &Frame, from_reach: bool) -> bool {
    let Some((_, original, _)) = crate::type_tool::layout(app, frame.layer) else { return false };
    let Some(st) = app.session.active() else { return false };
    let (document, revision) = (st.doc.id.0, st.revision);
    let session = session_of(app, frame);
    app.ui.type_transform = Some(TypeTransform { document, revision, original, session, from_reach });
    true
}

/// The layer transform the open box stands for: the layer's own while the box is still on the frame's corners as that
/// transform puts them (where a session starts), else the one that maps the frame onto the box. `None` for a box type
/// cannot take (collapsed, not finite, or too large to rasterize).
fn affine(t: &TypeTransform) -> Option<Affine> {
    let quad = t.session.quad();
    if Some(quad) == quad_of(t.session.rect(), &t.original) {
        return Some(t.original);
    }
    let r = t.session.rect();
    let [a, b, _, d] = quad;
    let (w, h) = (r.x1 - r.x0, r.y1 - r.y0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (ax, ay, bx, by) = ((b.x - a.x) / w, (b.y - a.y) / w, (d.x - a.x) / h, (d.y - a.y) / h);
    let out = Affine { m: [ax, ay, bx, by, a.x - ax * r.x0 - bx * r.y0, a.y - ay * r.x0 - by * r.y0] };
    let xs = quad.map(|p| p.x);
    let ys = quad.map(|p| p.y);
    let width = xs.into_iter().fold(f64::MIN, f64::max) - xs.into_iter().fold(f64::MAX, f64::min);
    let height = ys.into_iter().fold(f64::MIN, f64::max) - ys.into_iter().fold(f64::MAX, f64::min);
    // Reject collapsed/nonfinite geometry and bound input-sized text raster allocations.
    (out.m.iter().all(|v| v.is_finite())
        && out.determinant().abs() >= 1e-8
        && quad.iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 1e7 && p.y.abs() <= 1e7)
        && width >= 0.05
        && height >= 0.05
        && width * height <= 64.0 * 1024.0 * 1024.0)
        .then_some(out)
}

pub(crate) fn current_transform(app: &PhotocraftApp, id: LayerId) -> Option<Affine> {
    app.ui.text_edit.as_ref().filter(|ed| ed.layer == id.0)?;
    affine(app.ui.type_transform.as_ref()?)
}

/// Ends the transform and applies it: one `type.edit` with the new transform, or nothing when the box came back to
/// where it started.
pub(crate) fn finish(app: &mut PhotocraftApp) {
    cancel_stale(app);
    let Some(mut t) = app.ui.type_transform.take() else { return };
    // Ends a running box gesture. Whether anything changed is decided below, against the layer's own transform: the
    // session's untouched test compares with the unrotated frame.
    let _ = t.session.commit();
    cancel_drag(app);
    let pivot = t.session.pivot();
    app.ui.type_transform_pivot = Some([pivot.x, pivot.y]);
    let Some(transform) = affine(&t) else { return };
    // Pivot-only and out-and-back gestures leave no history entry, even in an untouched session. The pointer reaches
    // the box in single-precision screen points, so a box dragged back to where it started is compared by where its
    // corners are, to a thousandth of a document unit, not by exact matrix entries.
    let unchanged = match (quad_of(t.session.rect(), &transform), quad_of(t.session.rect(), &t.original)) {
        (Some(now), Some(then)) => now.iter().zip(then).all(|(a, b)| a.distance(b) < 1e-3),
        _ => false,
    };
    if unchanged {
        return;
    }
    let Some(ed) = app.ui.text_edit.as_ref() else { return };
    let params = json!({"layer": ed.layer, "transform": transform.m, "coalesce": ed.session});
    if app.run("type.edit", params).is_ok()
        && let Some(preview) = app.type_transform_preview.take().filter(|p| p.transform == transform)
    {
        crate::canvas::shown_as_document(app, preview.document, |key| key == preview.key);
    }
}

/// The transform box of the open edit, over the canvas `canvas`. With ⌘ held, or while a capture begun so runs, the
/// whole box takes the pointer; otherwise only its move reach does, so a plain press just outside the text moves it
/// without ending typing (#5956), and every other press is the editor's or the tool's. A press the box holds starts
/// the capture at once, before any movement; the capture ends with the press. A pass applies only the box's own events
/// of its own input: a second pass of a frame gets none, so a drag is never applied twice.
///
/// Called twice per pass: with `drawn` false before the text editor (the move reach sits under it, so a press on the
/// text is the editor's) and with `drawn` true after it (the whole box is above the editor). Each call does its own
/// half; both show the one box, under one id, so a press it holds carries over.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, canvas: egui::Rect, drawn: bool) {
    cancel_stale(app);
    if !app.ui.tool.is_type() || app.ui.text_edit.is_none() {
        return;
    }
    let ctx = ui.ctx().clone();
    let pointer = craft::ui::runtime::pointer::frame(&ctx);
    if whole_box(app, pointer.mods) != drawn {
        return;
    }
    let Some(frame) = frame(app) else { return };
    let spare;
    let session = match &app.ui.type_transform {
        Some(t) => &t.session,
        None => {
            spare = session_of(app, &frame);
            &spare
        }
    };
    let settings = settings();
    let reach = if drawn { TransformReach::Box } else { TransformReach::MoveOnly { points: MOVE_REACH_PT as f32 } };
    let out = TransformOverlay::new(Label::from(&*tl!("Type transform")), session, &settings)
        .reach(reach)
        .rotate_zone(RotateZone::border(ROTATE_REACH_PT))
        .mods(Some(pointer.mods))
        .id_salt("type transform")
        .show_at(ui, canvas);
    // The press that went down on the box in this pass, or the box's first edit, starts the capture.
    let pressed = out.held.is_some() && pointer.press.is_some();
    if app.ui.type_transform.is_none() && !pressed && out.edits.is_empty() && out.apply.is_none() && !out.discard {
        return;
    }
    if app.ui.type_transform.is_none() && !start(app, &frame, !drawn) {
        return;
    }
    let now = ui.input(|i| i.time);
    let (mut released, mut cancelled) = (false, false);
    let Some(t) = app.ui.type_transform.as_mut() else { return };
    for edit in out.edits {
        if let TransformEdit::Box(event) = &edit {
            released |= event.phase() == Some(GesturePhase::Commit);
            cancelled |= event.phase() == Some(GesturePhase::Cancel);
        }
        t.session.apply(edit, now);
    }
    let dragging = t.session.dragging();
    if out.discard {
        let _ = t.session.discard();
        cancel_drag(app);
    } else if released || out.apply.is_some() {
        finish(app);
    } else if cancelled && !dragging {
        cancel_drag(app);
    } else if out.held.is_none() && !dragging {
        // The press ended without a drag (a click), or a key moved the box: the capture ends with it.
        finish(app);
    }
}

/// The `typeTransform` of the control protocol: the open transform's document, revision, frame and reference point,
/// or null.
pub(crate) fn inspect(app: &PhotocraftApp) -> serde_json::Value {
    let Some(t) = app.ui.type_transform.as_ref() else { return serde_json::Value::Null };
    let point = |p: kurbo::Point| [p.x, p.y];
    let rect = t.session.rect();
    json!({
        "document": t.document,
        "revision": t.revision,
        "frame": {
            "layer": app.ui.text_edit.as_ref().map(|ed| ed.layer),
            "rect": [rect.x0, rect.y0, rect.x1, rect.y1],
            "quad": t.session.quad().map(point),
            "pivot": point(t.session.pivot()),
        },
    })
}

/// Preview via the existing text renderer; all glyphs/styles/effects remain native document data.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    cancel_stale(app);
    if !active(app) {
        if let Some(p) = app.type_transform_preview.as_mut() {
            // Retain damage bookkeeping for the return to the document, not its rendered tiles.
            p.shown = None;
        }
        return None;
    }
    let t = app.ui.type_transform.as_ref()?;
    let st = app.session.documents().get(idx)?;
    if st.doc.id.0 != t.document || st.revision != t.revision {
        return None;
    }
    let transform = affine(t)?;
    if transform == t.original {
        return None;
    }
    let (id, revision, source, layer) = (st.doc.id, st.revision, st.doc.clone(), LayerId(app.ui.text_edit.as_ref()?.layer));
    if let Some(p) = &app.type_transform_preview
        && p.document == id
        && p.revision == revision
        && p.transform == transform
        && let Some(shown) = &p.shown
    {
        return Some((shown.clone(), p.key));
    }
    let original_area = source.layer(layer)?.surface().map_or(Rect::EMPTY, |s| s.tile_bounds());
    let mut shown = (*source).clone();
    let LayerContent::Text(text) = &mut shown.layer_mut(layer)?.content else { return None };
    text.transform = transform;
    let started = crate::gpu_canvas::now_ms();
    photocraft_engine::type_cmds::refresh(&source, text);
    let area = text.cache.as_ref().map_or(Rect::EMPTY, |s| s.tile_bounds());
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&shown).unwrap_or(shown));
    let key = PREVIEW_BASE | (app.ui.alloc_id() & (PREVIEW_BASE - 1));
    let mut areas = app.type_transform_preview.take().filter(|p| p.document == id && p.revision == revision).map(|p| p.areas).unwrap_or_default();
    if areas.len() >= 128 {
        areas.pop_front();
    }
    areas.push_back((key, area));
    app.type_transform_preview = Some(Preview { document: id, revision, transform, shown: Some(shown.clone()), key, original_area, areas });
    app.perf.span("type transform preview", crate::gpu_canvas::now_ms() - started);
    Some((shown, key))
}

pub(crate) fn damage(app: &PhotocraftApp, doc: DocId, revision: u64, before: u64, after: u64) -> Option<Rect> {
    let p = app.type_transform_preview.as_ref().filter(|p| p.document == doc && p.revision == revision)?;
    let area = |key| if key == 0 { Some(p.original_area) } else { p.areas.iter().find(|(k, _)| *k == key).map(|(_, r)| *r) };
    Some(area(before)?.union(&area(after)?))
}
