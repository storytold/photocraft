//! Content-Aware Move › Transform On Drop (the options-bar checkbox, as in Photoshop): after the
//! drop a transform box sits on the moved patch. The box is the framework's `TransformOverlay` on a
//! `TransformSession`, limited to moving, scaling and rotating; ↩, ✓ or a double-click computes the
//! blend with the transformed patch (`paint.contentAwareMove` with `scaleX`, `scaleY` and `rotate`),
//! Esc or ⊘ drops the whole move. The document is untouched until then, so the move stays one
//! History step.

use craft::behavior::geometry::quad::BoxAllow;
use craft::behavior::kurbo::{Point, Rect};
use craft::draw::ui::transform::{ApplyBy, TransformOverlay, TransformSession};
use craft::imaging::ui::selection::{PointMap, SelectionOverlay};

use crate::PhotocraftApp;
use crate::canvas::outline::SurfaceMask;
use crate::state::{CamTransform, Tool};

/// Where the patch lands: the drag offset (px) and the scale (%, per axis) and clockwise rotation
/// (degrees) about its centre, as `paint.contentAwareMove` takes them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub offset: [i64; 2],
    pub scale: [f64; 2],
    pub rotate: f64,
}

impl Placement {
    /// The patch is only moved (or not even that).
    pub fn is_plain_move(&self) -> bool {
        self.scale == [100.0, 100.0] && self.rotate == 0.0
    }
}

/// Whether a Transform On Drop box is up.
pub(crate) fn active(app: &PhotocraftApp) -> bool {
    app.ui.cam_transform.is_some()
}

/// Put the box on the selection dropped `offset` away. False when there is no selection to box.
pub(crate) fn begin(app: &mut PhotocraftApp, offset: [i32; 2]) -> bool {
    let Some((document, revision, canvas, sel)) =
        app.session.active().and_then(|st| Some((st.doc.id.0, st.revision, st.doc.bounds(), st.doc.selection.clone()?)))
    else {
        return false;
    };
    let b = app.cached_bounds(u64::MAX - document, &sel).intersect(&canvas);
    if b.is_empty() {
        return false;
    }
    let [x0, y0, x1, y1] =
        [b.x0.saturating_add(offset[0]), b.y0.saturating_add(offset[1]), b.x1.saturating_add(offset[0]), b.y1.saturating_add(offset[1])].map(f64::from);
    // The patch's blend takes only a scale and a rotation: the box refuses skew, distort and warp.
    let session = TransformSession::<()>::new(Rect::new(x0, y0, x1, y1))
        .allow(BoxAllow::new(true, false, false).with_warp(false))
        .settings(&crate::transform_tool::settings(app), crate::transform_tool::default_interpolation(app));
    app.ui.cam_transform = Some(CamTransform { document, revision, offset, session });
    app.ui.status = crate::i18n::fmt(tl!("Scale or rotate the moved area, then press {key}"), &[("key", &crate::shortcuts::pretty("Enter"))]);
    app.ui.status_error = false;
    true
}

/// The box as `paint.contentAwareMove`'s params: its rect → quad map split into a scale along the
/// box's own axes, a rotation and the move of its centre. `None` for a collapsed or absurd box.
pub(crate) fn placement(t: &CamTransform) -> Option<Placement> {
    placement_of(t.offset, t.session.rect(), t.session.quad())
}

/// [`placement`] of a box `quad` over the source frame `rect`, for a box that is not the session's yet.
fn placement_of(offset: [i32; 2], rect: Rect, quad: [Point; 4]) -> Option<Placement> {
    let [x0, y0, x1, y1] = [rect.x0, rect.y0, rect.x1, rect.y1];
    let (w, h) = (x1 - x0, y1 - y0);
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let q = quad.map(|p| [p.x, p.y]);
    // The box's x axis (scaled by sx, turned) and y axis (scaled by sy, a negative sy flips).
    let u = [(q[1][0] - q[0][0]) / w, (q[1][1] - q[0][1]) / w];
    let v = [(q[3][0] - q[0][0]) / h, (q[3][1] - q[0][1]) / h];
    let angle = u[1].atan2(u[0]);
    let (sn, cs) = angle.sin_cos();
    let (sx, sy) = (u[0].hypot(u[1]), v[1] * cs - v[0] * sn);
    // Scale and rotation are about the selection's centre, then the offset moves it.
    let centre = [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0];
    let origin = [(x0 + x1) / 2.0 - f64::from(offset[0]), (y0 + y1) / 2.0 - f64::from(offset[1])];
    let off = [(centre[0] - origin[0]).round(), (centre[1] - origin[1]).round()];
    let round = |v: f64| {
        let r = (v * 1e4).round() / 1e4;
        if r == 0.0 { 0.0 } else { r }
    };
    let (sx, sy, rotate) = (round(sx * 100.0), round(sy * 100.0), round(angle.to_degrees()));
    let sane = |s: f64| (1.0..=10_000.0).contains(&s.abs());
    let ok = rotate.is_finite() && sane(sx) && sane(sy) && off.iter().all(|v| v.is_finite() && v.abs() < 1e7);
    ok.then_some(Placement { offset: [off[0] as i64, off[1] as i64], scale: [sx, sy], rotate })
}

/// Drops the box and its status hint.
pub(crate) fn cancel(app: &mut PhotocraftApp) {
    if app.ui.cam_transform.take().is_some() && !app.ui.status_error {
        // The box's hint is done with.
        app.ui.status.clear();
    }
}

/// Drop the box when what it was made for changed: another tool, document or edit.
pub(crate) fn cancel_stale(app: &mut PhotocraftApp) {
    let Some(t) = &app.ui.cam_transform else { return };
    let valid = app.ui.tool == Tool::ContentAwareMove
        && app.session.active().is_some_and(|st| st.doc.id.0 == t.document && st.revision == t.revision && st.doc.selection.is_some());
    if !valid {
        cancel(app);
    }
}

/// `paint.contentAwareMove`'s params for the box's placement and the options bar.
pub(crate) fn params(app: &PhotocraftApp, place: &Placement) -> serde_json::Value {
    let mut p = crate::retouch_ui::content_aware_move_params(app, place.offset);
    if !place.is_plain_move() {
        p["scaleX"] = place.scale[0].into();
        p["scaleY"] = place.scale[1].into();
        p["rotate"] = place.rotate.into();
    }
    p
}

/// ↩ / ✓: the move with the transformed patch, as one background job and History step.
pub(crate) fn commit(app: &mut PhotocraftApp) {
    cancel_stale(app);
    let Some(t) = app.ui.cam_transform.clone() else { return };
    cancel(app);
    let Some(place) = placement(&t) else { return };
    if place.offset == [0, 0] && place.is_plain_move() {
        crate::retouch_ui::patch_hint(app);
        return;
    }
    let p = params(app, &place);
    if let Err(e) = app.run("paint.contentAwareMove", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The selection's outline where the box puts it, and the box, in the canvas body after the document
/// display. The box takes every press in the body (the canvas is otherwise inert, as during Free
/// Transform); Enter, a double click or ✓ commit, Escape discards.
///
/// A second pass of a frame shows the box again from the session as the first pass left it and applies
/// only its own output; after a commit or discard there is no box and the pass returns at once.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    cancel_stale(app);
    if app.ui.cam_transform.is_none() {
        return;
    }
    let now = ui.input(|i| i.time);
    let settings = crate::transform_tool::settings(app);
    let out = {
        let Some(t) = app.ui.cam_transform.as_ref() else { return };
        // `cancel_stale` left a box only over the active document with a selection; the overlay traces the
        // selection and keeps the trace by the mask's revision.
        if let Some(st) = app.session.active()
            && let Some(selection) = st.doc.selection.as_ref()
            && let Some(h) = t.session.homography()
        {
            let mask = SurfaceMask::new(selection, selection.generation(), [st.doc.size.width, st.doc.size.height]);
            // A scale and rotate box has no horizon: the map always answers.
            SelectionOverlay::new(tl!("Content-Aware Move").to_owned(), &mask)
                .offset(t.offset)
                .mapped(PointMap::new(move |p: Point| Some(h.apply(p))))
                .show_at(ui, rect);
        }
        TransformOverlay::new(tl!("Content-Aware Move").to_owned(), &t.session, &settings).show_at(ui, rect)
    };
    if let Some(t) = app.ui.cam_transform.as_mut() {
        for edit in out.edits {
            // An absurd box (no placement) is refused: the session keeps the box it had.
            let mut proposed = t.session.clone();
            proposed.apply(edit, now);
            if placement_of(t.offset, proposed.rect(), proposed.quad()).is_some() {
                t.session = proposed;
            }
        }
        t.session.settle(now, false);
    }
    if out.discard {
        cancel(app);
    } else if matches!(out.apply, Some(ApplyBy::Key | ApplyBy::DoubleClick)) {
        // A click outside the box does nothing: only Enter, ✓ and a double click commit.
        commit(app);
    }
}
