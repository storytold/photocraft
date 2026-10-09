//! Crop tool gestures, Photoshop "Classic Mode" style (the frame moves and turns over a fixed
//! image):
//!
//! - with no frame, or inside the untouched default frame, a drag draws a new frame; ⇧ makes it
//!   square, ⌥ draws it from the centre, and holding Space while drawing repositions the frame
//!   being drawn;
//! - inside the frame, a drag moves it;
//! - on an edge or corner, a drag resizes it along the frame's own (possibly turned) axes; ⇧ keeps
//!   the frame's aspect ratio, ⌥ resizes about the centre, and an options-bar ratio preset always
//!   holds;
//! - outside the frame (off its handles) the pointer is the rotate cursor and a drag turns the
//!   frame about its centre, ⇧ in 15° steps, with the angle beside the pointer (#1792). As in
//!   Photoshop, a new frame is drawn after the pending one is committed or cancelled.
//!
//! ↵ commits (`image.crop`, see `canvas::commit_crop`, with the turn as its `angle`: the document
//! is rotated so the frame is upright, then cropped to it) and Esc cancels. The pending frame lives
//! in `UiState::crop_rect` (the frame before its turn) and `UiState::crop_angle` (degrees,
//! clockwise on screen), so the control channel reads both. Photoshop's default mode keeps the box
//! upright and turns the image behind it; the result is the same, but here the frame turns over
//! the image (its Classic Mode), which the canvas draws without rotating the view.
//!
//! As in Photoshop, from the first press until the crop is committed or cancelled the canvas also
//! shows the layers' pixels past its edges (kept by a crop with Delete Cropped Pixels off, or moved
//! out), with transparency out to the frame, under the shield ([`shows_beyond_canvas`]).

use egui::{CursorIcon, Modifiers};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// How far (screen points) from an edge the pointer still grabs it.
const HANDLE_PX: f64 = 8.0;
/// Smallest frame (document px) a gesture may leave behind.
const MIN_FRAME: f64 = 1.0;

/// Crop tool pointer state (not serialized: it only exists during a gesture).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CropState {
    pub drag: Option<CropDrag>,
    /// Space is held: drawing a new frame moves it instead of sizing it.
    pub space: bool,
    /// `UiState::crop_rect` is the untouched default frame ([`ensure_frame`]): a drag inside it
    /// draws a new frame (as in Photoshop) instead of moving it, and committing it does nothing.
    pub default_frame: bool,
    /// The document (index and size) the default frame was made for.
    frame_for: Option<(usize, u32, u32)>,
    /// The frame is being edited (pressed since it was made): the canvas shows what lies past it.
    pub editing: bool,
    /// The document the pending frame (or gesture) belongs to ([`cancel_stale`], #1918).
    doc: Option<photocraft_doc::DocId>,
}

/// A crop gesture in progress. Rects are `[x0, y0, x1, y1]` in document coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CropDrag {
    /// Drawing a new frame from `anchor`; `prev` is the frame it replaces, restored when the new
    /// one is too small (a click).
    Draw { anchor: [f64; 2], cur: [f64; 2], last: [f64; 2], prev: Option<[f64; 4]> },
    /// Moving the frame `rect` grabbed at `start`.
    Move { start: [f64; 2], rect: [f64; 4] },
    /// Dragging an edge or corner: `hx`/`hy` are -1 (left/top), 1 (right/bottom) or 0 (untouched).
    Resize { hx: i8, hy: i8, start: [f64; 2], rect: [f64; 4] },
    /// Turning the frame about `center`: the pointer's direction from it at the press (radians)
    /// and the frame's angle then (degrees).
    Rotate { center: [f64; 2], grab: f64, from: f64 },
}

/// What the pointer is over, relative to a crop frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Handle(i8, i8),
    Inside,
    Outside,
}

/// Hit-test a frame with `tol` document px of reach (shrunk for tiny frames so they stay movable).
pub fn hit(r: [f64; 4], p: [f64; 2], tol: f64) -> Hit {
    hit_axes(r, p, [tol; 2])
}

/// Per-axis reach in the frame's own coordinates; keep its middle movable even when tiny.
fn hit_axes(r: [f64; 4], p: [f64; 2], tol: [f64; 2]) -> Hit {
    let axis = |lo: f64, hi: f64, v: f64, other_in: bool, tol: f64| -> i8 {
        let t = tol.min((hi - lo) / 4.0).max(0.0);
        // Outside the frame the full reach applies; inside it, the shrunk one.
        let near = |e: f64, outward: f64| (v - e) * outward <= tol && (e - v) * outward <= t;
        match (other_in, near(lo, -1.0), near(hi, 1.0)) {
            (false, ..) => 0,
            (_, true, true) => {
                if (v - lo).abs() <= (v - hi).abs() {
                    -1
                } else {
                    1
                }
            }
            (_, true, false) => -1,
            (_, false, true) => 1,
            _ => 0,
        }
    };
    let in_x = p[0] >= r[0] - tol[0] && p[0] <= r[2] + tol[0];
    let in_y = p[1] >= r[1] - tol[1] && p[1] <= r[3] + tol[1];
    let (hx, hy) = (axis(r[0], r[2], p[0], in_y, tol[0]), axis(r[1], r[3], p[1], in_x, tol[1]));
    if hx != 0 || hy != 0 {
        Hit::Handle(hx, hy)
    } else if p[0] > r[0] && p[0] < r[2] && p[1] > r[1] && p[1] < r[3] {
        Hit::Inside
    } else {
        Hit::Outside
    }
}

/// The centre of `[x0, y0, x1, y1]`.
pub fn center(r: [f64; 4]) -> [f64; 2] {
    [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0]
}

/// `p` turned `deg` degrees clockwise on screen (y down) about `c`.
pub fn turn(p: [f64; 2], c: [f64; 2], deg: f64) -> [f64; 2] {
    let (s, k) = deg.to_radians().sin_cos();
    let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
    [c[0] + k * dx - s * dy, c[1] + s * dx + k * dy]
}

/// The corners of frame `r` turned `deg` degrees about its centre: top-left, top-right,
/// bottom-right, bottom-left (of the upright frame).
pub fn corners(r: [f64; 4], deg: f64) -> [[f64; 2]; 4] {
    let c = center(r);
    [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]].map(|p| turn(p, c, deg))
}

/// The axis-aligned bounds `[x0, y0, x1, y1]` of frame `r` turned `deg` degrees.
pub fn turned_bounds(r: [f64; 4], deg: f64) -> [f64; 4] {
    if deg == 0.0 {
        return r;
    }
    corners(r, deg)
        .iter()
        .fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// [`hit`] for frame `r` turned `deg` degrees about its centre: handles and inside are found in
/// the frame's own axes.
pub fn hit_turned(r: [f64; 4], deg: f64, p: [f64; 2], tol: f64) -> Hit {
    if deg == 0.0 { hit(r, p, tol) } else { hit(r, turn(p, center(r), -deg), tol) }
}

/// An angle in degrees brought into (-180, 180]; 0 for anything not finite.
pub fn normalized_angle(deg: f64) -> f64 {
    if !deg.is_finite() {
        return 0.0;
    }
    let a = deg.rem_euclid(360.0);
    if a > 180.0 { a - 360.0 } else { a }
}

/// The frame's angle after turning it from `from` degrees by the pointer's sweep about `c` from
/// direction `grab` (radians) to `p`; ⇧ snaps it to 15° steps, as in Photoshop.
pub fn turned_angle(c: [f64; 2], grab: f64, from: f64, p: [f64; 2], snap: bool) -> f64 {
    let now = (p[1] - c[1]).atan2(p[0] - c[0]);
    let a = from + (now - grab).to_degrees();
    normalized_angle(if snap { (a / 15.0).round() * 15.0 } else { a })
}

/// [`resized`] for frame `r` turned `deg` degrees: the handle moves along the frame's own axes by
/// the document-space drag `d`, and the edges opposite stay where they are on the page.
pub fn resized_turned(r: [f64; 4], deg: f64, hx: i8, hy: i8, d: [f64; 2], ratio: Option<f64>, alt: bool) -> [f64; 4] {
    if deg == 0.0 {
        return resized(r, hx, hy, d, ratio, alt);
    }
    let c = center(r);
    // In the frame's axes (about its old centre), then the new centre back on the page.
    let n = resized(r, hx, hy, turn(d, [0.0, 0.0], -deg), ratio, alt);
    let nc = turn(center(n), c, deg);
    let (hw, hh) = ((n[2] - n[0]) / 2.0, (n[3] - n[1]) / 2.0);
    [nc[0] - hw, nc[1] - hh, nc[0] + hw, nc[1] + hh]
}

/// `[x0, y0, x1, y1]` with x0 ≤ x1, y0 ≤ y1.
fn normalized(a: [f64; 2], b: [f64; 2]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
}

fn size_ok(r: [f64; 4]) -> bool {
    r[2] - r[0] >= MIN_FRAME && r[3] - r[1] >= MIN_FRAME
}

/// The options-bar ratio preset as width / height.
fn preset_ratio(app: &PhotocraftApp) -> Option<f64> {
    let size = app.session.active().map_or((1.0, 1.0), |s| (s.doc.size.width as f64, s.doc.size.height as f64));
    crate::chrome_ui::crop_ratio(&app.ui.tool_options.crop_ratio, size.0, size.1).map(|(w, h)| w / h).filter(|k| k.is_finite() && *k > 0.0)
}

/// A new frame drawn from `anchor` to `cur`: ⇧ square, ⌥ from the centre, `ratio` held.
pub fn drawn(anchor: [f64; 2], cur: [f64; 2], ratio: Option<f64>, mods: Modifiers) -> [f64; 4] {
    let e = match ratio {
        Some(k) => crate::chrome_ui::marquee_end("fixedRatio", k, 1.0, false, anchor, cur),
        None => crate::chrome_ui::marquee_end("", 0.0, 0.0, mods.shift, anchor, cur),
    };
    if mods.alt { normalized([2.0 * anchor[0] - e[0], 2.0 * anchor[1] - e[1]], e) } else { normalized(anchor, e) }
}

/// Frame `r` with handle (`hx`, `hy`) dragged by `d`. `alt` resizes about the centre; `ratio`
/// (width / height) keeps the proportions.
pub fn resized(r: [f64; 4], hx: i8, hy: i8, d: [f64; 2], ratio: Option<f64>, alt: bool) -> [f64; 4] {
    // Per axis: the fixed point, the signed extent from it, and whether it is centred (the
    // extent is then a half-extent).
    let axis = |lo: f64, hi: f64, h: i8, d: f64| -> (f64, f64, bool) {
        let c = (lo + hi) / 2.0;
        match h {
            1 if alt => (c, hi + d - c, true),
            1 => (lo, hi + d - lo, false),
            -1 if alt => (c, lo + d - c, true),
            -1 => (hi, lo + d - hi, false),
            _ => (c, (hi - lo) / 2.0, true),
        }
    };
    let (fx, mut ex, cx) = axis(r[0], r[2], hx, d[0]);
    let (fy, mut ey, cy) = axis(r[1], r[3], hy, d[1]);
    if let Some(k) = ratio.filter(|k| k.is_finite() && *k > 0.0) {
        let full = |e: f64, c: bool| e.abs() * if c { 2.0 } else { 1.0 };
        let (w, h) = (full(ex, cx), full(ey, cy));
        let (w, h) = match (hx != 0, hy != 0) {
            // A corner: the larger relative extent wins.
            (true, true) => {
                let s = (w / k).max(h);
                (s * k, s)
            }
            (true, false) => (w, w / k),
            (false, true) => (h * k, h),
            (false, false) => (w, h),
        };
        let back = |e: f64, len: f64, c: bool| if e < 0.0 { -1.0 } else { 1.0 } * if c { len / 2.0 } else { len };
        ex = back(ex, w, cx);
        ey = back(ey, h, cy);
    }
    let span = |f: f64, e: f64, c: bool| if c { (f - e, f + e) } else { (f, f + e) };
    let ((ax, bx), (ay, by)) = (span(fx, ex, cx), span(fy, ey, cy));
    normalized([ax, ay], [bx, by])
}

/// Space is held (set by the canvas each frame, or by `ui.pointer`'s `space` flag).
pub fn set_space(app: &mut PhotocraftApp, down: bool) {
    app.crop.space = down;
}

/// As in Photoshop, the Crop tool always shows a frame: on picking the tool, and after a crop is
/// cancelled or committed, it frames the selection's bounds, or the whole canvas.
pub fn ensure_frame(app: &mut PhotocraftApp) {
    if app.ui.tool != Tool::Crop {
        app.crop.editing = false;
        // Leaving the tool drops an untouched default frame (a drawn one stays pending).
        if app.crop.default_frame {
            app.ui.crop_rect = None;
            app.crop.default_frame = false;
        }
        if app.ui.crop_rect.is_none() {
            app.ui.crop_angle = 0.0;
        }
        return;
    }
    // No frame (cancelled or committed): the next one starts upright.
    if app.ui.crop_rect.is_none() && app.crop.drag.is_none() {
        app.ui.crop_angle = 0.0;
    }
    let key = app.session.active_index().zip(app.session.active()).map(|(i, st)| (i, st.doc.size.width, st.doc.size.height));
    // Another document (or a resized one) gets its own default frame.
    if app.crop.default_frame && app.crop.frame_for != key && app.crop.drag.is_none() {
        app.ui.crop_rect = None;
    }
    if app.ui.crop_rect.is_some() || app.crop.drag.is_some() {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let canvas = st.doc.bounds();
    // Computed once per new frame, not per frame drawn.
    let r = st.doc.selection.as_ref().map(|s| s.content_bounds().intersect(&canvas)).filter(|b| !b.is_empty()).unwrap_or(canvas);
    if r.is_empty() {
        return;
    }
    app.ui.crop_rect = Some([f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)]);
    app.crop.default_frame = true;
    app.crop.frame_for = key;
    app.crop.editing = false;
    claim(app);
}

/// A crop gesture is in progress: Space repositions the frame rather than panning.
pub fn active(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.crop.drag.is_some()
}

/// Cancel the pending crop: the frame, any gesture on it and the default-frame state. With the
/// Crop tool still picked, [`ensure_frame`] then makes the active document a new default frame.
pub fn cancel(app: &mut PhotocraftApp) {
    app.ui.crop_rect = None;
    app.ui.crop_angle = 0.0;
    app.crop.drag = None;
    app.crop.default_frame = false;
    app.crop.frame_for = None;
    app.crop.editing = false;
}

/// The pending frame belongs to the active document.
fn claim(app: &mut PhotocraftApp) {
    app.crop.doc = app.session.active().map(|st| st.doc.id);
}

/// A crop is pending: the Crop tool's frame on the active document has been set (drawn, moved,
/// resized or turned), not just the untouched default frame the tool shows.
pub fn pending(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop
        && app.ui.crop_rect.is_some()
        && !app.crop.default_frame
        && (app.crop.doc.is_none() || app.crop.doc == app.session.active().map(|st| st.doc.id))
}

/// Menu commands Photoshop greys out while a crop is pending (#1918), until it is committed or
/// cancelled. Checked item by item in Photoshop 27.11: everything except Close, Save, Save As,
/// Save a Copy; Undo, Redo, Toggle Last State, Search; Image › Crop (which commits the frame); the
/// View menu but Proof Setup, Pixel Aspect Ratio (Correction), 32-bit Preview Options, Flip
/// Horizontal and Screen Mode; the Window menu but Workspace and Adjustments; Help but System Info.
pub fn blocks(app: &PhotocraftApp, id: &str) -> bool {
    pending(app) && !available_while_pending(id)
}

fn available_while_pending(id: &str) -> bool {
    if let Some(v) = id.strip_prefix("view.") {
        let greyed = ["proofSetup.", "pixelAspectRatio", "thirtyTwoBitPreviewOptions", "flipHorizontal", "screenMode."];
        return !greyed.iter().any(|g| v.starts_with(g));
    }
    if let Some(w) = id.strip_prefix("window.") {
        return !(w.starts_with("workspace.") || w == "panel.adjustments");
    }
    if id.starts_with("help.") {
        return id != "help.systemInfo";
    }
    matches!(
        id,
        "file.close"
            | "file.save"
            | "file.saveAs"
            | "file.saveACopy"
            | "file.exit"
            | "edit.undo"
            | "edit.redo"
            | "edit.toggleLastState"
            | "edit.search"
            | "image.crop"
    )
}

/// A pending crop belongs to the document it was drawn on (#1918): when another document becomes
/// active (File › New, Open, a tab switch, closing the document) it is cancelled, so it never
/// shows on or crops another document. (As in Photoshop: switching to another document discards
/// the pending crop, and that document gets the tool's own frame.) Run on every frame and command
/// (`sync_views`) and before a commit.
pub fn cancel_stale(app: &mut PhotocraftApp) {
    let active = app.session.active().map(|st| st.doc.id);
    let pending = app.ui.crop_rect.is_some() || app.crop.drag.is_some();
    // A frame made before any document was claimed (control channel, tests) is the active one's.
    if pending && app.crop.doc.is_some() && app.crop.doc != active {
        cancel(app);
    }
    app.crop.doc = active;
}

/// The frame is being edited, so the canvas shows the pixels past its edges (Photoshop's crop
/// preview): from the first press with the tool until the crop is committed or cancelled.
pub fn shows_beyond_canvas(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.ui.crop_rect.is_some() && (app.crop.editing || app.crop.drag.is_some())
}

/// The button went down on the canvas with the Crop tool: the frame is being edited from now on.
/// Returns true when that is new.
pub fn press(app: &mut PhotocraftApp) -> bool {
    let new = app.ui.tool == Tool::Crop && app.ui.crop_rect.is_some() && !app.crop.editing;
    if new {
        app.crop.editing = true;
    }
    new
}

pub(crate) fn hit_screen(app: &PhotocraftApp, r: [f64; 4], p: [f64; 2]) -> Hit {
    let deg = angle(app);
    let metric = crate::canvas::ScreenMetric::active(app);
    let reach = metric.reach(HANDLE_PX);
    let (s, c) = deg.to_radians().sin_cos();
    // A local edge normal n maps to D^-T n on screen, where D is the aspect/zoom
    // scale. Its norm converts screen perpendicular distance to local-axis reach.
    // Camera rotation and flip preserve that norm; rotating the point recovers frame axes.
    let tol = [(c * reach[0]).hypot(s * reach[1]), (s * reach[0]).hypot(c * reach[1])];
    let local = turn(p, center(r), -deg);
    let hit = hit_axes(r, local, tol);
    let Hit::Handle(hx, hy) = hit else { return hit };
    let quad = corners(r, deg);
    // Intersecting infinite edge strips can reach far beyond an acute projected corner.
    // Bound outside hits by the finite boundary; sqrt(2) retains the old 8-by-8 corner reach.
    let corner_reach = HANDLE_PX * std::f64::consts::SQRT_2;
    let outside = local[0] < r[0] || local[0] > r[2] || local[1] < r[1] || local[1] > r[3];
    if outside && !metric.near_quad(quad, p, corner_reach) {
        return Hit::Outside;
    }
    let [a, b, c, d] = quad;
    let (corner, x_edge, y_edge) = match (hx, hy) {
        (-1, -1) => (a, (a, d), (a, b)),
        (1, -1) => (b, (b, c), (a, b)),
        (1, 1) => (c, (b, c), (c, d)),
        (-1, 1) => (d, (a, d), (c, d)),
        _ => return hit,
    };
    if metric.distance(corner, p) <= corner_reach {
        return hit;
    }
    // Inside an acute corner, both strips also overlap far from its vertex: grab the
    // nearer finite edge there rather than resizing two axes from an invisible corner handle.
    let distance = |(a, b)| {
        let (a, b, p) = (metric.point(a), metric.point(b), metric.point(p));
        let d = [b[0] - a[0], b[1] - a[1]];
        let len = d[0] * d[0] + d[1] * d[1];
        let t = if len > 0.0 { ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len } else { 0.0 }.clamp(0.0, 1.0);
        (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
    };
    if distance(x_edge) <= distance(y_edge) { Hit::Handle(hx, 0) } else { Hit::Handle(0, hy) }
}

/// The pending frame's angle (degrees), 0 when it isn't a finite number.
pub fn angle(app: &PhotocraftApp) -> f64 {
    normalized_angle(app.ui.crop_angle)
}

/// A press at document point `p` would turn the frame (Crop tool, outside the frame and its
/// handles) rather than draw, move or resize one.
pub fn turns_at(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.ui.tool == Tool::Crop && app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite())).is_some_and(|r| hit_screen(app, r, p) == Hit::Outside)
}

/// A double-click at document point `p` commits the crop (Photoshop: double-click inside the box,
/// like ↵): Crop tool, a pending frame, and `p` inside it (not on a handle, not outside).
pub fn commits_at(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.ui.tool == Tool::Crop
        && app.crop.drag.is_none()
        && app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite())).is_some_and(|r| hit_screen(app, r, p) == Hit::Inside)
}

/// The frame is being turned: its angle, for the readout beside the pointer.
pub fn turning(app: &PhotocraftApp) -> Option<f64> {
    matches!(app.crop.drag, Some(CropDrag::Rotate { .. })).then(|| angle(app))
}

/// Pointer input for the Crop tool. Returns true when the event was its.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    if app.ui.tool != Tool::Crop {
        app.crop.drag = None;
        return false;
    }
    let p = match ev {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => [x, y],
    };
    if app.session.active().is_none() || !p[0].is_finite() || !p[1].is_finite() {
        return true;
    }
    match ev {
        ToolEvent::Down { .. } => {
            claim(app);
            app.crop.editing = true;
            let frame = app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite()));
            let deg = angle(app);
            app.crop.drag = Some(match frame.map(|r| (r, hit_screen(app, r, p))) {
                Some((rect, Hit::Handle(hx, hy))) => CropDrag::Resize { hx, hy, start: p, rect },
                Some((rect, Hit::Inside)) if !app.crop.default_frame => CropDrag::Move { start: p, rect },
                // Outside the frame a drag turns it, as in Photoshop (#1792).
                Some((rect, Hit::Outside)) => {
                    let c = center(rect);
                    CropDrag::Rotate { center: c, grab: (p[1] - c[1]).atan2(p[0] - c[0]), from: deg }
                }
                _ => {
                    app.ui.crop_angle = 0.0;
                    CropDrag::Draw { anchor: p, cur: p, last: p, prev: app.ui.crop_rect }
                }
            });
        }
        ToolEvent::Move { .. } => update(app, p, mods),
        ToolEvent::Up { .. } => {
            update(app, p, mods);
            let Some(drag) = app.crop.drag.take() else { return true };
            let ok = app.ui.crop_rect.is_some_and(size_ok);
            // A click outside the untouched default frame (no turn) leaves it the default.
            let changed = !matches!(drag, CropDrag::Rotate { from, .. } if from == angle(app));
            if ok && changed {
                app.crop.default_frame = false;
            }
            match drag {
                CropDrag::Draw { prev, .. } if !ok => app.ui.crop_rect = prev,
                CropDrag::Move { rect, .. } | CropDrag::Resize { rect, .. } if !ok => app.ui.crop_rect = Some(rect),
                _ => {}
            }
        }
    }
    true
}

/// The frame follows the pointer at `p`.
fn update(app: &mut PhotocraftApp, p: [f64; 2], mods: Modifiers) {
    let ratio = preset_ratio(app);
    let space = app.crop.space;
    let deg = angle(app);
    if let Some(CropDrag::Rotate { center, grab, from }) = app.crop.drag {
        app.ui.crop_angle = turned_angle(center, grab, from, p, mods.shift);
        return;
    }
    let Some(drag) = app.crop.drag.as_mut() else { return };
    let rect = match drag {
        CropDrag::Draw { anchor, cur, last, .. } => {
            if space {
                // Space: the frame being drawn follows the pointer, keeping its size.
                let d = [p[0] - last[0], p[1] - last[1]];
                *anchor = [anchor[0] + d[0], anchor[1] + d[1]];
                *cur = [cur[0] + d[0], cur[1] + d[1]];
            } else {
                *cur = p;
            }
            *last = p;
            if *anchor == *cur {
                return;
            }
            drawn(*anchor, *cur, ratio, mods)
        }
        CropDrag::Move { start, rect } => {
            let (dx, dy) = (p[0] - start[0], p[1] - start[1]);
            [rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy]
        }
        CropDrag::Resize { hx, hy, start, rect } => {
            let keep = || {
                let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
                (w > 0.0 && h > 0.0).then(|| w / h)
            };
            let ratio = ratio.or_else(|| if mods.shift { keep() } else { None });
            resized_turned(*rect, deg, *hx, *hy, [p[0] - start[0], p[1] - start[1]], ratio, mods.alt)
        }
        CropDrag::Rotate { .. } => return,
    };
    app.ui.crop_rect = Some(rect);
}

/// Cursor over the canvas at document point `p`: resize arrows on the edges, move inside.
pub fn cursor(app: &PhotocraftApp, p: [f64; 2]) -> Option<CursorIcon> {
    if app.ui.tool != Tool::Crop {
        return None;
    }
    let deg = angle(app);
    let h = match app.crop.drag {
        Some(CropDrag::Move { .. }) => Hit::Inside,
        Some(CropDrag::Resize { hx, hy, .. }) => Hit::Handle(hx, hy),
        Some(CropDrag::Draw { .. }) => return Some(CursorIcon::Crosshair),
        Some(CropDrag::Rotate { .. }) => Hit::Outside,
        None => hit_screen(app, app.ui.crop_rect?, p),
    };
    Some(match h {
        Hit::Handle(hx, hy) => resize_icon(hx, hy, deg, crate::canvas::ViewXform::active(app)),
        Hit::Inside => CursorIcon::Move,
        // Photoshop's curved two-headed arrow (`draw_turn_cursor`, from `turn_cursor_dir`).
        Hit::Outside => CursorIcon::None,
    })
}

/// Where the turn cursor's arc bends at document point `p` (pointer outside the frame, or turning
/// it): towards the frame corner of `p`'s quadrant, along the diagonal of the frame's own axes.
/// As in Photoshop that gives four cursors, one per corner; they turn with the frame.
pub fn turn_cursor_dir(app: &PhotocraftApp, p: [f64; 2]) -> Option<[f64; 2]> {
    if app.ui.tool != Tool::Crop {
        return None;
    }
    let r = app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite()))?;
    let deg = angle(app);
    let turning = match app.crop.drag {
        Some(CropDrag::Rotate { .. }) => true,
        Some(_) => false,
        None => hit_screen(app, r, p) == Hit::Outside,
    };
    if !turning || !(p[0].is_finite() && p[1].is_finite()) {
        return None;
    }
    let c = center(r);
    let local = turn(p, c, -deg);
    let sx = if local[0] < c[0] { 1.0 } else { -1.0 };
    let sy = if local[1] < c[1] { 1.0 } else { -1.0 };
    Some(turn([sx * std::f64::consts::FRAC_1_SQRT_2, sy * std::f64::consts::FRAC_1_SQRT_2], [0.0, 0.0], deg))
}

/// Photoshop's crop turn cursor at screen point `p`: a short arc with an arrowhead at each end,
/// curving round the frame corner that lies in screen direction `toward` (an original drawing, no
/// proprietary cursor asset).
pub fn draw_turn_cursor(ctx: &egui::Context, p: egui::Pos2, toward: egui::Vec2) {
    let len = toward.length();
    if !len.is_finite() || len < 1e-6 {
        return;
    }
    let u = toward / len;
    let tokens = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("crop-turn-cursor")));
    const R: f32 = 12.0;
    const HALF: f32 = 0.75;
    let c = p + u * R;
    let mid = (-u.y).atan2(-u.x);
    let at = |a: f32| c + egui::vec2(a.cos(), a.sin()) * R;
    let points: Vec<egui::Pos2> = (0..=16).map(|i| at(mid - HALF + 2.0 * HALF * i as f32 / 16.0)).collect();
    painter.add(egui::Shape::line(points.clone(), egui::Stroke::new(3.5, tokens.shadow)));
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, tokens.accent_text)));
    for (a, sign) in [(mid - HALF, -1.0f32), (mid + HALF, 1.0)] {
        let tip = at(a) + egui::vec2(-a.sin(), a.cos()) * (sign * 2.0);
        let tangent = egui::vec2(-a.sin(), a.cos()) * sign;
        let normal = egui::vec2(a.cos(), a.sin());
        painter.add(egui::Shape::convex_polygon(
            vec![tip + tangent * 3.5, tip - tangent * 3.0 + normal * 3.5, tip - tangent * 3.0 - normal * 3.5],
            tokens.accent_text,
            egui::Stroke::new(1.0, tokens.shadow),
        ));
    }
}

/// The resize arrow for handle (`hx`, `hy`) of a frame turned `deg` degrees: the handle's outward
/// direction, turned with the frame, to the nearest of the four arrows.
fn resize_icon(hx: i8, hy: i8, deg: f64, xf: Option<crate::canvas::ViewXform>) -> CursorIcon {
    let mut d = turn([f64::from(hx), f64::from(hy)], [0.0, 0.0], deg);
    if let Some(xf) = xf {
        let screen = xf.map_vec(egui::vec2(d[0] as f32, d[1] as f32));
        d = [f64::from(screen.x), f64::from(screen.y)];
    }
    let a = d[1].atan2(d[0]).rem_euclid(std::f64::consts::PI);
    match ((a / std::f64::consts::FRAC_PI_4).round() as i64).rem_euclid(4) {
        0 => CursorIcon::ResizeHorizontal,
        1 => CursorIcon::ResizeNwSe,
        2 => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeNeSw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projected_app(aspect: f32, ppp: f32, crop_angle: f64, camera: f32, flip: bool) -> PhotocraftApp {
        let mut a = app(SampleType::U8);
        a.ui.crop_rect = Some([100.0, 100.0, 900.0, 700.0]);
        a.ui.crop_angle = crop_angle;
        a.ui.view.pixel_aspect = format!("custom:{aspect}");
        a.ui.view.pixel_aspect_correction = true;
        a.ui.view.flip_horizontal = flip;
        a.ui.views = vec![crate::state::View::default()];
        a.ui.views[0].zoom = ppp;
        a.ui.views[0].zoom /= ppp;
        a.ui.views[0].center = [500.0, 400.0];
        a.ui.views[0].rotation = camera;
        a.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
        a
    }

    /// Construct the pointer in the rendered edge's perpendicular direction, independently of
    /// the native handle tolerance. Camera rotation, reflection and DPI affect the real input.
    fn edge_pointer(a: &PhotocraftApp, hx: i8, hy: i8, distance: f32) -> [f64; 2] {
        let r = a.ui.crop_rect.unwrap();
        let c = center(r);
        let deg = a.ui.crop_angle;
        let at = turn([c[0] + f64::from(hx) * 400.0, c[1] + f64::from(hy) * 300.0], c, deg);
        let tangent = turn(if hx == 0 { [1.0, 0.0] } else { [0.0, 1.0] }, [0.0; 2], deg);
        let outward = turn([f64::from(hx), f64::from(hy)], [0.0; 2], deg);
        let xf = crate::canvas::ViewXform::active(a).unwrap();
        let t = xf.map_vec(egui::vec2(tangent[0] as f32, tangent[1] as f32));
        let out = xf.map_vec(egui::vec2(outward[0] as f32, outward[1] as f32));
        let mut normal = egui::vec2(-t.y, t.x).normalized();
        if normal.dot(out) < 0.0 {
            normal = -normal;
        }
        xf.to_doc(xf.to_screen(at[0] as f32, at[1] as f32) + normal * distance)
    }

    #[test]
    fn rotated_aspect_corners_have_finite_screen_reach() {
        let mut failures = Vec::new();
        for aspect in [0.1, 10.0] {
            for ppp in [1.0, 2.0] {
                for deg in [45.0, 90.0] {
                    for (camera, flip) in [(0.0, false), (37.0, true)] {
                        for (hx, hy) in [(-1, -1), (1, -1), (1, 1), (-1, 1)] {
                            for distance in [2.0, 7.0, 20.0, 70.0] {
                                let mut a = projected_app(aspect, ppp, deg, camera, flip);
                                let r = a.ui.crop_rect.unwrap();
                                let c = center(r);
                                let corner = turn([c[0] + f64::from(hx) * 400.0, c[1] + f64::from(hy) * 300.0], c, deg);
                                // Pan this corner to the canvas centre so its full outside reach is visible.
                                a.ui.views[0].center = [corner[0] as f32, corner[1] as f32];
                                let xf = crate::canvas::ViewXform::active(&a).unwrap();
                                let out = turn([f64::from(hx), f64::from(hy)], [0.0; 2], deg);
                                let out = xf.map_vec(egui::vec2(out[0] as f32, out[1] as f32)).normalized();
                                let p = xf.to_doc(xf.to_screen(corner[0] as f32, corner[1] as f32) + out * distance);
                                let expected = if distance <= 7.0 { Hit::Handle(hx, hy) } else { Hit::Outside };
                                let got = hit_screen(&a, r, p);
                                if got != expected {
                                    failures.push(format!("PAR {aspect}, DPI {ppp}, crop {deg}, camera {camera}, flip {flip}, corner {hx}/{hy}, {distance}pt: {got:?} != {expected:?}"));
                                }
                                tool_event(&mut a, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, NONE);
                                assert!(
                                    matches!((got, a.crop.drag), (Hit::Handle(x, y), Some(CropDrag::Resize { hx, hy, .. })) if x == hx && y == hy)
                                        || matches!((got, a.crop.drag), (Hit::Outside, Some(CropDrag::Rotate { .. })))
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{} wrong corner picks:\n{}", failures.len(), failures.join("\n"));
    }

    #[test]
    fn rotated_aspect_acute_corner_extensions_do_not_grab_the_vertex() {
        let a = projected_app(10.0, 1.0, 45.0, 0.0, false);
        let r = a.ui.crop_rect.unwrap();
        let p = turn([905.0, 95.0], center(r), 45.0);
        assert_eq!(hit_screen(&a, r, p), Hit::Outside, "70.7 points beyond both finite edges");
    }

    #[test]
    fn rotated_aspect_inside_acute_corner_selects_the_near_edge() {
        let a = projected_app(10.0, 1.0, 45.0, 0.0, false);
        let r = a.ui.crop_rect.unwrap();
        let p = turn([895.0, 105.0], center(r), 45.0);
        assert!(
            matches!(hit_screen(&a, r, p), Hit::Handle(1, 0) | Hit::Handle(0, -1)),
            "inside the acute cone near both edges, but 70.7 points from the corner"
        );
    }

    #[test]
    fn square_pixel_crop_keeps_the_full_outside_corner_reach() {
        let a = projected_app(1.0, 1.0, 0.0, 0.0, false);
        let r = a.ui.crop_rect.unwrap();
        assert_eq!(hit_screen(&a, r, [908.0, 92.0]), Hit::Handle(1, -1));
        assert_eq!(hit_screen(&a, r, [108.0, 108.0]), Hit::Handle(-1, -1));
    }

    #[test]
    fn rotated_aspect_pointer_reach_tracks_visible_edges() {
        let mut failures = Vec::new();
        for aspect in [0.1, 10.0] {
            for ppp in [1.0, 2.0] {
                for deg in [45.0, 90.0] {
                    for camera in [0.0, 37.0] {
                        for flip in [false, true] {
                            for (hx, hy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                                for distance in [-20.0, -2.0, 2.0, 20.0] {
                                    let mut a = projected_app(aspect, ppp, deg, camera, flip);
                                    let p = edge_pointer(&a, hx, hy, distance);
                                    let r = a.ui.crop_rect.unwrap();
                                    let expected = if distance.abs() == 2.0 {
                                        Hit::Handle(hx, hy)
                                    } else if distance < 0.0 {
                                        Hit::Inside
                                    } else {
                                        Hit::Outside
                                    };
                                    let got = hit_screen(&a, r, p);
                                    if got != expected {
                                        failures.push(format!("PAR {aspect}, DPI {ppp}, crop {deg}, camera {camera}, flip {flip}, edge {hx}/{hy}, {distance}pt: {got:?} != {expected:?}"));
                                    }
                                    assert_eq!(turns_at(&a, p), got == Hit::Outside);
                                    assert_eq!(commits_at(&a, p), got == Hit::Inside);
                                    let history = a.session.active().unwrap().history.past_len();
                                    tool_event(&mut a, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, NONE);
                                    let selected = match a.crop.drag.unwrap() {
                                        CropDrag::Resize { hx, hy, .. } => Hit::Handle(hx, hy),
                                        CropDrag::Move { .. } => Hit::Inside,
                                        CropDrag::Rotate { .. } => Hit::Outside,
                                        other => panic!("unexpected gesture: {other:?}"),
                                    };
                                    assert_eq!(selected, got, "pointer dispatch uses the hover hit");
                                    assert_eq!(a.session.active().unwrap().history.past_len(), history);
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{} wrong picks:\n{}", failures.len(), failures.join("\n"));
    }

    #[test]
    fn rotated_aspect_resize_cursor_follows_visible_direction() {
        for (aspect, deg, camera, flip, expected) in [
            (0.1, 45.0, 0.0, false, CursorIcon::ResizeVertical),
            (10.0, 45.0, 0.0, false, CursorIcon::ResizeHorizontal),
            (1.0, 45.0, 37.0, false, CursorIcon::ResizeVertical),
            (1.0, 90.0, 37.0, false, CursorIcon::ResizeNeSw),
            (1.0, 90.0, 37.0, true, CursorIcon::ResizeNwSe),
        ] {
            let mut a = projected_app(aspect, 2.0, deg, camera, flip);
            // The cursor remains correct while resizing even when the pointer leaves the frame.
            a.crop.drag = Some(CropDrag::Resize { hx: 1, hy: 0, start: [0.0; 2], rect: a.ui.crop_rect.unwrap() });
            assert_eq!(cursor(&a, [0.0; 2]), Some(expected), "PAR {aspect}, crop {deg}, camera {camera}, flip {flip}");
        }
    }

    #[test]
    fn rotated_aspect_resize_keeps_native_geometry_and_history() {
        for aspect in [0.1, 10.0] {
            for ppp in [1.0, 2.0] {
                for deg in [45.0, 90.0] {
                    for (camera, flip) in [(0.0, false), (37.0, true)] {
                        let mut a = projected_app(aspect, ppp, deg, camera, flip);
                        let old = a.ui.crop_rect.unwrap();
                        let p = edge_pointer(&a, 1, 0, 2.0);
                        let delta = turn([12.0, 0.0], [0.0; 2], deg);
                        let end = [p[0] + delta[0], p[1] + delta[1]];
                        let history = a.session.active().unwrap().history.past_len();
                        drag(&mut a, &[p, end], NONE);
                        let new = a.ui.crop_rect.unwrap();
                        assert!(
                            (new[2] - new[0] - 812.0).abs() < 1e-6 && (new[3] - new[1] - 600.0).abs() < 1e-6,
                            "PAR {aspect}, DPI {ppp}, crop {deg}: {new:?}"
                        );
                        let (before, after) = (corners(old, deg), corners(new, deg));
                        for i in [0, 3] {
                            assert!(before[i].iter().zip(after[i]).all(|(a, b)| (a - b).abs() < 1e-6), "the opposite edge stays fixed");
                        }
                        assert_eq!(a.ui.crop_angle, deg);
                        assert!(a.crop.drag.is_none());
                        let st = a.session.active().unwrap();
                        assert_eq!((st.doc.size, st.history.past_len()), (Size::new(200, 100), history), "the gesture edits only the pending frame");
                    }
                }
            }
        }
    }

    #[test]
    fn rotated_aspect_canvas_button_grabs_the_visible_edge() {
        use egui::{Event, PointerButton};
        for ppp in [1.0, 2.0] {
            let mut a = projected_app(0.1, ppp, 90.0, 37.0, true);
            a.ui.extras.rulers = false;
            a.ui.views[0].pixel_aspect = 0.1;
            a.ui.views[0].fit_pending = false;
            a.ui.views[0].doc_size = [200, 100];
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 1000.0)).with_pixels_per_point(ppp).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    app.last_canvas_rect = ui.max_rect();
                    let v = app.ui.views[0].clone();
                    app.ui.views[0] = crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
                },
                a,
            );
            h.run_steps(2);
            let p = edge_pointer(h.state(), 0, -1, 2.0);
            let xf = crate::canvas::ViewXform::active(h.state()).unwrap();
            let screen = xf.to_screen(p[0] as f32, p[1] as f32);
            h.event(Event::PointerMoved(screen));
            h.event(Event::PointerButton { pos: screen, button: PointerButton::Primary, pressed: true, modifiers: NONE });
            h.step();
            h.event(Event::PointerMoved(screen + egui::vec2(10.0, 0.0)));
            h.step();
            assert!(matches!(h.state().crop.drag, Some(CropDrag::Resize { hx: 0, hy: -1, .. })), "DPI {ppp}: {:?}", h.state().crop.drag);
        }
    }

    #[test]
    #[ignore = "requires a wgpu adapter; captures the rotated PAR crop frame"]
    fn rotated_aspect_crop_visuals() {
        let dir = std::env::var_os("PHOTOCRAFT_ASPECT_SNAPSHOTS").map(std::path::PathBuf::from).expect("snapshot directory");
        std::fs::create_dir_all(&dir).unwrap();
        for aspect in [0.1, 10.0] {
            for ppp in [1.0, 2.0] {
                let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 900.0)).with_pixels_per_point(ppp).wgpu().build_eframe(move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                    let mut a = projected_app(aspect, ppp, 45.0, 37.0, true);
                    a.ui.extras.rulers = false;
                    a.ui.views[0].fit_pending = false;
                    a.ui.views[0].doc_size = [200, 100];
                    a.ui.views[0].zoom = ppp / aspect.max(1.0);
                    a
                });
                h.run_steps(5);
                h.render().unwrap().save(dir.join(format!("rotated-crop-par{aspect}-dpi{ppp}.png"))).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "requires a wgpu adapter; captures pointer reach at an acute crop corner"]
    fn rotated_aspect_crop_corner_visuals() {
        let dir = std::env::var_os("PHOTOCRAFT_ASPECT_SNAPSHOTS").map(std::path::PathBuf::from).expect("snapshot directory");
        std::fs::create_dir_all(&dir).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 900.0)).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut a = projected_app(10.0, 1.0, 45.0, 0.0, false);
            let corner = turn([900.0, 100.0], center(a.ui.crop_rect.unwrap()), 45.0);
            a.ui.extras.rulers = false;
            a.ui.views[0].fit_pending = false;
            a.ui.views[0].doc_size = [200, 100];
            a.ui.views[0].center = [corner[0] as f32, corner[1] as f32];
            a
        });
        h.run_steps(5);
        let xf = crate::canvas::ViewXform::active(h.state()).unwrap();
        let corner = turn([900.0, 100.0], center(h.state().ui.crop_rect.unwrap()), 45.0);
        let corner = xf.to_screen(corner[0] as f32, corner[1] as f32);
        for distance in [2.0, 7.0, 20.0, 70.0] {
            let screen = corner + egui::vec2(distance, 0.0);
            h.event(egui::Event::PointerMoved(screen));
            h.run_steps(2);
            let p = xf.to_doc(screen);
            assert_eq!(turns_at(h.state(), p), distance > 7.0);
            h.render().unwrap().save(dir.join(format!("acute-crop-corner-{distance}pt.png"))).unwrap();
        }
    }

    #[test]
    fn aspect_review_crop_gesture_picks_screen_handle() {
        for aspect in [0.1, 10.0] {
            for ppp in [1.0, 2.0] {
                for rotation in [0.0, 37.0] {
                    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                    a.run("file.new", serde_json::json!({"width":1000,"height":500})).unwrap();
                    a.ui.tool = crate::state::Tool::Crop;
                    a.ui.crop_rect = Some([100.0, 100.0, 900.0, 400.0]);
                    a.ui.view.pixel_aspect = format!("custom:{aspect}");
                    a.ui.view.pixel_aspect_correction = true;
                    a.ui.views[0].zoom = ppp;
                    a.ui.views[0].zoom /= ppp;
                    a.ui.views[0].rotation = rotation;
                    let xf = crate::canvas::ViewXform::active(&a).unwrap();
                    let at = xf.to_screen(100.0, 100.0);
                    let p = xf.to_doc(at + egui::vec2(-2.0, -2.0));
                    pointer(&mut a, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::NONE);
                    assert!(matches!(a.crop.drag, Some(CropDrag::Resize { hx: -1, hy: -1, .. })), "{aspect} {rotation}: {:?}", a.crop.drag);
                    let p = xf.to_doc(at + egui::vec2(-30.0, -30.0));
                    assert!(!matches!(hit_screen(&a, a.ui.crop_rect.unwrap(), p), Hit::Handle(-1, -1)));
                }
            }
        }
    }

    use crate::canvas::tool_event;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
    use serde_json::json;

    const NONE: Modifiers = Modifiers::NONE;
    const SHIFT: Modifiers = Modifiers::SHIFT;

    fn app(depth: SampleType) -> PhotocraftApp {
        let doc = Document::with_background("crop", Size::new(200, 100), ColorMode::Rgb, depth, Color::WHITE);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = Tool::Crop;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app
    }

    fn drag(app: &mut PhotocraftApp, pts: &[[f64; 2]], mods: Modifiers) {
        let (first, last) = (pts[0], pts[pts.len() - 1]);
        tool_event(app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, mods);
        for p in &pts[1..] {
            tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, mods);
        }
        tool_event(app, ToolEvent::Up { x: last[0], y: last[1] }, mods);
    }

    #[test]
    fn picking_the_tool_frames_the_canvas_or_the_selection() {
        // #668: the Crop tool started with no frame.
        let mut app = app(SampleType::U8);
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 200.0, 100.0]));
        // Committing the untouched frame crops nothing.
        let steps = app.session.active().unwrap().history.past_len();
        crate::canvas::commit_crop(&mut app);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        assert_eq!(app.session.active().unwrap().doc.size, Size::new(200, 100));
        // A drag inside the untouched frame draws a new one instead of moving it.
        ensure_frame(&mut app);
        drag(&mut app, &[[50.0, 20.0], [80.0, 40.0], [120.0, 70.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([50.0, 20.0, 120.0, 70.0]));
        // Now it's a real frame: a drag inside moves it.
        drag(&mut app, &[[60.0, 30.0], [70.0, 30.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([60.0, 20.0, 130.0, 70.0]));
        // Cancelling frames the selection's bounds when there is one.
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        app.ui.crop_rect = None;
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([10.0, 10.0, 40.0, 30.0]));
        // Leaving the tool drops the untouched frame.
        app.ui.tool = Tool::Brush;
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, None);
    }

    #[test]
    fn enter_on_the_selection_frame_crops_to_it_at_8_and_16_bit() {
        // #1789: selection, C, ↵ did nothing, as the untouched frame was always treated as a no-op.
        for depth in [SampleType::U8, SampleType::U16] {
            let mut app = app(depth);
            app.run("select.rect", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap();
            ensure_frame(&mut app);
            assert_eq!(app.ui.crop_rect, Some([10.0, 20.0, 40.0, 60.0]));
            crate::canvas::commit_crop(&mut app);
            let doc = &app.session.active().unwrap().doc;
            assert_eq!(doc.size, Size::new(30, 40), "{depth:?}");
            assert_eq!(doc.depth, depth);
            assert!(doc.selection.is_none(), "cropping to the selection deselects, as in Photoshop");
            // The new default frame is the whole (cropped) canvas: ↵ on it does nothing.
            ensure_frame(&mut app);
            assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 30.0, 40.0]));
            let steps = app.session.active().unwrap().history.past_len();
            crate::canvas::commit_crop(&mut app);
            let st = app.session.active().unwrap();
            assert_eq!((st.doc.size, st.history.past_len()), (Size::new(30, 40), steps), "{depth:?}");
        }
    }

    #[test]
    fn the_frame_is_edited_from_a_press_until_commit_cancel_or_another_tool() {
        let mut app = app(SampleType::U8);
        ensure_frame(&mut app);
        assert!(!shows_beyond_canvas(&app), "a new frame is not being edited");
        assert!(press(&mut app));
        assert!(shows_beyond_canvas(&app));
        assert!(!press(&mut app), "only the first press is news");
        // A click on the frame keeps it edited.
        drag(&mut app, &[[50.0, 50.0]], NONE);
        assert!(shows_beyond_canvas(&app));
        // Esc drops the frame: the new default one isn't edited.
        app.ui.crop_rect = None;
        ensure_frame(&mut app);
        assert!(!shows_beyond_canvas(&app));
        // A drawn frame is edited until it is committed.
        drag(&mut app, &[[10.0, 10.0], [60.0, 40.0]], NONE);
        assert!(shows_beyond_canvas(&app));
        crate::canvas::commit_crop(&mut app);
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 50.0, 30.0]));
        assert!(!shows_beyond_canvas(&app));
        // Or until another tool is picked.
        drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
        app.ui.tool = Tool::Brush;
        ensure_frame(&mut app);
        assert!(!app.crop.editing && !shows_beyond_canvas(&app));
        assert!(!press(&mut app), "no frame without the Crop tool");
    }

    #[test]
    fn hit_test_finds_handles_inside_and_outside() {
        let r = [10.0, 10.0, 110.0, 60.0];
        assert_eq!(hit(r, [10.0, 10.0], 4.0), Hit::Handle(-1, -1));
        assert_eq!(hit(r, [112.0, 61.0], 4.0), Hit::Handle(1, 1));
        assert_eq!(hit(r, [60.0, 8.0], 4.0), Hit::Handle(0, -1));
        assert_eq!(hit(r, [108.0, 30.0], 4.0), Hit::Handle(1, 0));
        assert_eq!(hit(r, [60.0, 30.0], 4.0), Hit::Inside);
        assert_eq!(hit(r, [150.0, 30.0], 4.0), Hit::Outside);
        // A tiny frame keeps a movable middle.
        assert_eq!(hit([0.0, 0.0, 4.0, 4.0], [2.0, 2.0], 8.0), Hit::Inside);
    }

    #[test]
    fn dragging_inside_moves_the_frame() {
        let mut app = app(SampleType::U8);
        drag(&mut app, &[[20.0, 20.0], [80.0, 60.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 80.0, 60.0]));
        drag(&mut app, &[[50.0, 40.0], [60.0, 45.0], [70.0, 50.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]), "moved, not redrawn");
        // A click outside keeps the frame (and leaves it upright).
        drag(&mut app, &[[5.0, 5.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]));
        assert_eq!(app.ui.crop_angle, 0.0);
        // Outside, a drag turns the frame rather than drawing a new one (#1792).
        drag(&mut app, &[[150.0, 50.0], [190.0, 90.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]));
        assert!(app.ui.crop_angle > 10.0, "{}", app.ui.crop_angle);
    }

    #[test]
    fn dragging_outside_turns_the_frame_about_its_centre() {
        let mut app = app(SampleType::U8);
        app.ui.crop_rect = Some([40.0, 30.0, 100.0, 70.0]);
        // Centre (70, 50): from straight right to straight below is a quarter turn clockwise.
        drag(&mut app, &[[150.0, 50.0], [140.0, 70.0], [70.0, 95.0]], NONE);
        assert!((app.ui.crop_angle - 90.0).abs() < 1e-9, "{}", app.ui.crop_angle);
        assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]), "turning keeps the frame");
        // Back the other way, past -180 (normalised).
        drag(&mut app, &[[70.0, 120.0], [-10.0, 50.0], [70.0, -20.0]], NONE);
        assert!((app.ui.crop_angle - -90.0).abs() < 1e-9, "{}", app.ui.crop_angle);
        // ⇧ snaps the angle to 15° steps.
        app.ui.crop_angle = 0.0;
        let a = 22.0f64.to_radians();
        drag(&mut app, &[[150.0, 50.0], [70.0 + 80.0 * a.cos(), 50.0 + 80.0 * a.sin()]], SHIFT);
        assert_eq!(app.ui.crop_angle, 15.0);
        assert_eq!(cursor(&app, [150.0, 50.0]), Some(CursorIcon::None), "outside: the drawn turn cursor");
        // Turning the untouched default frame makes it a real frame; a click outside it doesn't.
        let mut app = super::tests::app(SampleType::U8);
        ensure_frame(&mut app);
        drag(&mut app, &[[250.0, 50.0]], NONE);
        assert!(app.crop.default_frame, "a click outside keeps the default frame");
        drag(&mut app, &[[250.0, 50.0], [250.0, 80.0]], NONE);
        assert!(!app.crop.default_frame && app.ui.crop_angle > 0.0);
        // Esc (no frame) starts the next one upright.
        app.ui.crop_rect = None;
        ensure_frame(&mut app);
        assert_eq!((app.ui.crop_rect, app.ui.crop_angle), (Some([0.0, 0.0, 200.0, 100.0]), 0.0));
    }

    #[test]
    fn hit_testing_a_turned_frame_uses_its_own_axes() {
        let r = [40.0, 30.0, 100.0, 70.0];
        // A quarter turn: the frame covers x 50..90, y 20..80 on the page.
        assert_eq!(hit_turned(r, 90.0, [70.0, 75.0], 2.0), Hit::Inside, "inside the turned frame only");
        assert_eq!(hit(r, [70.0, 75.0], 2.0), Hit::Outside);
        assert_eq!(hit_turned(r, 90.0, [45.0, 50.0], 2.0), Hit::Outside, "inside the upright frame only: the rotate zone");
        // The upright frame's top-left corner turns to the top-right of the page.
        assert_eq!(hit_turned(r, 90.0, [90.0, 20.0], 2.0), Hit::Handle(-1, -1));
        // Its right edge's middle is now at the bottom.
        assert_eq!(hit_turned(r, 90.0, [70.0, 80.0], 2.0), Hit::Handle(1, 0));
        let c = corners(r, 90.0);
        for (a, b) in c.iter().zip([[90.0, 20.0], [90.0, 80.0], [50.0, 80.0], [50.0, 20.0]]) {
            assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "{c:?}");
        }
        let b = turned_bounds(r, 90.0);
        assert!((b[0] - 50.0).abs() < 1e-9 && (b[3] - 80.0).abs() < 1e-9, "{b:?}");
        // Handle arrows turn with the frame.
        assert_eq!(resize_icon(1, 0, 0.0, None), CursorIcon::ResizeHorizontal);
        assert_eq!(resize_icon(1, 0, 90.0, None), CursorIcon::ResizeVertical);
        assert_eq!(resize_icon(1, 0, 45.0, None), CursorIcon::ResizeNwSe);
        assert_eq!(resize_icon(-1, -1, 90.0, None), CursorIcon::ResizeNeSw);
        assert_eq!(normalized_angle(540.0), 180.0);
        assert_eq!(normalized_angle(-190.0), 170.0);
        assert_eq!(normalized_angle(f64::NAN), 0.0);
    }

    #[test]
    fn resizing_a_turned_frame_moves_along_its_axes_and_keeps_the_opposite_edge() {
        let r = [40.0, 30.0, 100.0, 70.0];
        // At 90°, the right edge (local +x) faces down the page: dragging down 10 grows the width.
        let n = resized_turned(r, 90.0, 1, 0, [0.0, 10.0], None, false);
        assert!(((n[2] - n[0]) - 70.0).abs() < 1e-9 && ((n[3] - n[1]) - 40.0).abs() < 1e-9, "{n:?}");
        // The opposite (left) edge stays put on the page: at y = 20 with x 50..90.
        let (old, new) = (corners(r, 90.0), corners(n, 90.0));
        for (i, (a, b)) in [(0, 0), (3, 3)].map(|(i, j)| (old[i], new[j])).iter().enumerate() {
            assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "corner {i}: {a:?} vs {b:?}");
        }
        // A sideways drag does nothing to that handle.
        let same = resized_turned(r, 90.0, 1, 0, [25.0, 0.0], None, false);
        assert!(same.iter().zip(r).all(|(a, b)| (a - b).abs() < 1e-9), "{same:?}");
        // At 0° it is the plain resize.
        assert_eq!(resized_turned(r, 0.0, 1, 1, [5.0, 6.0], None, false), resized(r, 1, 1, [5.0, 6.0], None, false));
        // Through the pointer: grabbing the turned right-edge handle and dragging down 10.
        let mut app = app(SampleType::U8);
        app.ui.crop_rect = Some(r);
        app.ui.crop_angle = 90.0;
        drag(&mut app, &[[70.0, 80.0], [70.0, 90.0]], NONE);
        let got = app.ui.crop_rect.unwrap();
        assert!(got.iter().zip(n).all(|(a, b)| (a - b).abs() < 1e-9), "{got:?} vs {n:?}");
        // Moving a turned frame is a plain translation, angle kept.
        drag(&mut app, &[[70.0, 50.0], [80.0, 55.0]], NONE);
        let moved = app.ui.crop_rect.unwrap();
        assert!(moved.iter().zip([n[0] + 10.0, n[1] + 5.0, n[2] + 10.0, n[3] + 5.0]).all(|(a, b)| (a - b).abs() < 1e-9), "{moved:?}");
        assert_eq!(app.ui.crop_angle, 90.0);
    }

    /// ↵ on a turned frame crops the turned document (one step), at 8, 16 and 32 bits.
    #[test]
    fn commit_crops_to_the_turned_frame() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut app = app(depth);
            app.run("select.rect", json!({"x": 60, "y": 40, "width": 1, "height": 1})).unwrap();
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            let past = app.session.active().unwrap().history.past_len();
            // A 40 × 20 frame centred on (60, 40), turned 90° by a drag outside it.
            app.ui.crop_rect = Some([40.0, 30.0, 80.0, 50.0]);
            app.crop.default_frame = false;
            drag(&mut app, &[[120.0, 40.0], [60.0, 100.0]], NONE);
            assert!((app.ui.crop_angle - 90.0).abs() < 1e-6, "{depth:?}: {}", app.ui.crop_angle);
            crate::canvas::commit_crop(&mut app);
            assert!(app.ui.crop_rect.is_none());
            assert_eq!(app.ui.crop_angle, 0.0, "the next frame starts upright");
            let st = app.session.active().unwrap();
            assert_eq!(st.history.past_len(), past + 1, "{depth:?}: one undo step");
            assert_eq!((st.doc.size.width, st.doc.size.height), (40, 20), "{depth:?}");
            assert_eq!(st.doc.depth, depth);
            // The red pixel was at the frame's centre: it is at the cropped image's centre.
            let surf = st.doc.layers[0].surface().unwrap();
            let red = (19..=21).flat_map(|x| (9..=11).map(move |y| (x, y))).map(|(x, y)| surf.rgba(x, y)).find(|px| px[0] > 0.6 && px[1] < 0.4);
            assert!(red.is_some(), "{depth:?}: no red at the centre");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc.size, Size::new(200, 100), "{depth:?}");
        }
    }

    #[test]
    fn moving_the_frame_snaps_its_edges() {
        let mut app = app(SampleType::U8);
        app.ui.extras.snap = true;
        app.ui.crop_rect = Some([10.0, 10.0, 50.0, 40.0]);
        drag(&mut app, &[[30.0, 25.0], [23.0, 25.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([0.0, 10.0, 40.0, 40.0]), "left edge snaps to the document edge");
    }

    #[test]
    fn handles_resize_with_shift_and_alt() {
        let mut app = app(SampleType::U8);
        app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
        // Right edge.
        drag(&mut app, &[[100.0, 40.0], [120.0, 50.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 120.0, 60.0]));
        // Bottom-right corner, free.
        drag(&mut app, &[[120.0, 60.0], [140.0, 70.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 140.0, 70.0]));
        // ⇧ keeps 120:50 from the top-left corner: the larger relative extent wins.
        app.ui.crop_rect = Some([20.0, 20.0, 140.0, 70.0]);
        drag(&mut app, &[[20.0, 20.0], [10.0, 0.0]], SHIFT);
        let r = app.ui.crop_rect.unwrap();
        assert_eq!((r[2], r[3]), (140.0, 70.0), "opposite corner fixed");
        assert!(((r[2] - r[0]) / (r[3] - r[1]) - 120.0 / 50.0).abs() < 1e-9, "{r:?}");
        assert_eq!(r[3] - r[1], 70.0);
        // ⌥ resizes about the centre.
        app.ui.crop_rect = Some([40.0, 20.0, 80.0, 60.0]);
        drag(&mut app, &[[80.0, 40.0], [90.0, 40.0]], Modifiers::ALT);
        assert_eq!(app.ui.crop_rect, Some([30.0, 20.0, 90.0, 60.0]));
        // An options-bar ratio holds on an edge drag, centred on the other axis.
        app.ui.tool_options.crop_ratio = "1:1".into();
        app.ui.crop_rect = Some([40.0, 20.0, 80.0, 60.0]);
        drag(&mut app, &[[80.0, 40.0], [100.0, 40.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([40.0, 10.0, 100.0, 70.0]));
    }

    #[test]
    fn dragging_a_handle_past_the_other_side_flips_and_collapse_reverts() {
        let mut app = app(SampleType::U8);
        app.ui.crop_rect = Some([20.0, 20.0, 60.0, 60.0]);
        drag(&mut app, &[[60.0, 40.0], [10.0, 40.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([10.0, 20.0, 20.0, 60.0]));
        // Collapsing to nothing restores the frame the gesture started from.
        drag(&mut app, &[[20.0, 40.0], [10.0, 40.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([10.0, 20.0, 20.0, 60.0]));
    }

    #[test]
    fn space_repositions_the_frame_being_drawn() {
        let mut app = app(SampleType::U8);
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, NONE);
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 40.0, pressure: 1.0 }, NONE);
        assert!(active(&app));
        set_space(&mut app, true);
        tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 50.0, pressure: 1.0 }, NONE);
        assert_eq!(app.ui.crop_rect, Some([30.0, 20.0, 70.0, 50.0]), "same size, moved");
        set_space(&mut app, false);
        tool_event(&mut app, ToolEvent::Move { x: 90.0, y: 60.0, pressure: 1.0 }, NONE);
        tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 60.0 }, NONE);
        assert_eq!(app.ui.crop_rect, Some([30.0, 20.0, 90.0, 60.0]), "sizing resumes from the moved anchor");
        assert!(!active(&app));
    }

    /// Real egui input on the canvas: holding Space mid-draw moves the frame instead of panning.
    #[test]
    fn space_on_the_canvas_moves_the_frame_not_the_view() {
        use egui::{Event, Key, PointerButton, pos2};
        let mut a = app(SampleType::U8);
        let view = crate::state::View {
            zoom: 2.0,

            pixel_aspect: 1.0,
            center: [100.0, 50.0],
            fit_pending: false,
            fill_pending: false,
            doc_size: [200, 100],
            rotation: 0.0,
        };
        a.ui.views = vec![view.clone()];
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 400.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let v = app.ui.views[0].clone();
                crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
            },
            a,
        );
        h.run_steps(2);
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        let p0 = pos2(200.0, 150.0);
        h.event(Event::PointerMoved(p0));
        h.event(button(p0, true));
        h.step();
        for p in [pos2(220.0, 170.0), pos2(260.0, 190.0)] {
            h.event(Event::PointerMoved(p));
            h.step();
        }
        let drawn = h.state().ui.crop_rect.unwrap();
        // The frame starts where the button went down (200, 150), not at the first move past
        // egui's drag threshold (#123).
        assert_eq!((drawn[2] - drawn[0], drawn[3] - drawn[1]), (30.0, 20.0), "60x40 screen px at 200%");
        h.event(space(true));
        h.step();
        for p in [pos2(280.0, 200.0), pos2(300.0, 210.0)] {
            h.event(Event::PointerMoved(p));
            h.step();
        }
        let moved = h.state().ui.crop_rect.unwrap();
        assert_eq!(moved, [drawn[0] + 20.0, drawn[1] + 10.0, drawn[2] + 20.0, drawn[3] + 10.0], "moved by 40x20 screen px");
        h.event(space(false));
        h.event(button(pos2(300.0, 210.0), false));
        h.step();
        let st = h.state();
        assert_eq!(st.ui.crop_rect, Some(moved));
        assert_eq!(st.ui.views[0].center, view.center, "the view did not pan");
        assert!(st.crop.drag.is_none());
    }

    /// #1918: a pending crop belongs to the document it was drawn on. File › New, opening a file
    /// or switching tabs cancels it; it never shows on, or crops, another document.
    /// A turned pending frame is dropped with its angle when another document becomes active, and
    /// Esc drops both too, so the next frame starts upright.
    #[test]
    fn cancelling_a_turned_crop_resets_its_angle() {
        let mut app = app(SampleType::U8);
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        app.ui.crop_angle = 30.0;
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        app.sync_views();
        assert_eq!(app.ui.crop_angle, 0.0);
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        app.ui.crop_angle = -12.5;
        cancel(&mut app);
        assert_eq!((app.ui.crop_rect, app.ui.crop_angle, app.crop.drag), (None, 0.0, None));
    }

    #[test]
    fn a_pending_crop_stays_with_its_document() {
        let size = |app: &PhotocraftApp| app.session.active().map(|st| st.doc.size);
        // File › New.
        let mut app = app(SampleType::U8);
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([10.0, 10.0, 50.0, 40.0]));
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        assert_eq!(app.ui.crop_rect, None, "the first document's frame is gone");
        assert!(app.crop.drag.is_none() && !app.crop.editing && !app.crop.default_frame);
        crate::canvas::commit_crop(&mut app);
        assert_eq!(size(&app), Some(Size::new(300, 150)), "↵ crops nothing");
        // The new document gets its own default frame.
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 300.0, 150.0]));
        assert!(app.crop.default_frame);
        // Switching tabs (outside a command), even before the next frame's sync: ↵ never crops
        // the other document.
        drag(&mut app, &[[20.0, 20.0], [60.0, 60.0]], NONE);
        assert!(app.session.set_active(0));
        crate::canvas::commit_crop(&mut app);
        assert_eq!(size(&app), Some(Size::new(200, 100)));
        assert_eq!(app.session.documents()[1].doc.size, Size::new(300, 150));
        assert_eq!(app.ui.crop_rect, None);
        // The next frame's sync also drops a frame left on a tab switched away from.
        drag(&mut app, &[[20.0, 20.0], [60.0, 60.0]], NONE);
        assert!(app.session.set_active(1));
        app.sync_views();
        assert_eq!(app.ui.crop_rect, None);
        // Closing another document keeps the pending crop; closing its own cancels it.
        drag(&mut app, &[[20.0, 20.0], [60.0, 60.0]], NONE);
        app.session.close(0);
        app.sync_views();
        assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 60.0, 60.0]), "still its document");
        crate::canvas::commit_crop(&mut app);
        assert_eq!(size(&app), Some(Size::new(40, 40)));
        drag(&mut app, &[[5.0, 5.0], [25.0, 25.0]], NONE);
        app.session.close(0);
        app.sync_views();
        assert!(app.session.active().is_none());
        assert_eq!(app.ui.crop_rect, None);
        assert!(app.crop.drag.is_none());
        crate::canvas::commit_crop(&mut app);
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, None, "no document, no frame");
    }

    /// A drag in progress when its document goes away is dropped too, not finished elsewhere.
    #[test]
    fn a_crop_drag_ends_with_its_document() {
        let mut app = app(SampleType::U8);
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, NONE);
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 40.0, pressure: 1.0 }, NONE);
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 50.0 }, NONE);
        assert_eq!(app.ui.crop_rect, None);
        crate::canvas::commit_crop(&mut app);
        assert_eq!(app.session.active().map(|st| st.doc.size), Some(Size::new(300, 150)));
    }

    /// #1918: as in Photoshop, a pending crop (not the untouched default frame) greys almost every
    /// menu command; the ones Photoshop keeps stay available, and commit or cancel frees the rest.
    /// Walks the whole menu catalog.
    #[test]
    fn a_pending_crop_greys_the_menus_photoshop_greys() {
        let mut app = app(SampleType::U8);
        ensure_frame(&mut app);
        assert!(app.crop.default_frame);
        let ids: Vec<&str> = crate::menu_catalog::CATALOG.iter().map(|e| e.3).filter(|id| *id != "---").collect();
        for id in &ids {
            assert!(!blocks(&app, id), "default frame: {id} stays as it was");
        }
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        assert!(pending(&app));
        let kept = [
            "file.close",
            "file.save",
            "file.saveAs",
            "file.saveACopy",
            "edit.undo",
            "edit.redo",
            "edit.toggleLastState",
            "edit.search",
            "image.crop",
            "view.proofColors",
            "view.gamutWarning",
            "view.zoomIn",
            "view.zoomOut",
            "view.fitOnScreen",
            "view.actualPixels",
            "view.twoHundredPercent",
            "view.printSize",
            "view.extras",
            "view.show.grid",
            "view.rulers",
            "view.snap",
            "view.snapTo.guides",
            "view.lockSlices",
            "view.newGuide",
            "window.arrange.tile",
            "window.panel.layers",
            "help.about",
        ];
        let greyed = [
            "file.new",
            "file.open",
            "file.openAs",
            "file.closeAll",
            "file.revert",
            "file.export.exportAs",
            "file.placeEmbedded",
            "file.automate.batch",
            "file.scripts.browse",
            "file.import.notes",
            "file.fileInfo",
            "file.print",
            "edit.cut",
            "edit.fill",
            "edit.freeTransform",
            "edit.colorSettings",
            "edit.preferences.general",
            "image.imageSize",
            "image.mode.grayscale",
            "layer.new.layer",
            "select.all",
            "filter.blur.gaussianBlur",
            "view.proofSetup.workingCmyk",
            "view.pixelAspectRatio.square",
            "view.pixelAspectRatioCorrection",
            "view.thirtyTwoBitPreviewOptions",
            "view.flipHorizontal",
            "view.screenMode.fullScreen",
            "window.workspace.essentials",
            "window.panel.adjustments",
            "help.systemInfo",
        ];
        for id in kept {
            assert!(!blocks(&app, id), "{id} stays available");
        }
        for id in greyed {
            assert!(blocks(&app, id), "{id} is greyed");
            assert!(!crate::menus::is_enabled(&app, id), "{id} greyed in the menus");
            assert!(!crate::menus::modal_allows(&app, id), "{id} refused over the menu gate");
        }
        assert!(
            ids.iter()
                .filter(|id| id.starts_with("layer.") || id.starts_with("type.") || id.starts_with("select.") || id.starts_with("filter."))
                .all(|id| blocks(&app, id))
        );
        // Committing still runs `image.crop` itself, and frees the menus again.
        crate::canvas::commit_crop(&mut app);
        assert_eq!(app.session.active().unwrap().doc.size, Size::new(40, 30));
        assert!(!pending(&app) && crate::menus::is_enabled(&app, "file.new"));
        // Cancelling does too.
        drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
        assert!(blocks(&app, "file.new"));
        cancel(&mut app);
        assert!(!blocks(&app, "file.new"));
        // Another tool: nothing is pending.
        drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
        app.ui.tool = Tool::Brush;
        assert!(!blocks(&app, "file.new"));
    }

    /// Image › Crop with a pending crop commits the frame (the one Image item Photoshop keeps).
    #[test]
    fn image_crop_menu_commits_a_pending_crop() {
        let mut app = app(SampleType::U8);
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "image.crop", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.size, Size::new(40, 30));
        assert!(app.ui.crop_rect.is_none() || app.crop.default_frame);
    }

    #[test]
    fn commit_crops_to_the_moved_frame_at_8_and_16_bit() {
        for depth in [SampleType::U8, SampleType::U16] {
            let mut app = app(depth);
            app.run("select.rect", json!({"x": 60, "y": 40, "width": 1, "height": 1})).unwrap();
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
            drag(&mut app, &[[30.0, 25.0], [70.0, 45.0]], NONE);
            assert_eq!(app.ui.crop_rect, Some([50.0, 30.0, 90.0, 60.0]));
            crate::canvas::commit_crop(&mut app);
            assert!(app.ui.crop_rect.is_none());
            let doc = &app.session.active().unwrap().doc;
            assert_eq!((doc.size.width, doc.size.height), (40, 30), "{depth:?}");
            assert_eq!(doc.depth, depth);
            let px = doc.layers[0].surface().unwrap().rgba(10, 10);
            assert!(px[0] > 0.99 && px[1] < 0.01, "{depth:?} {px:?}");
        }
    }

    #[test]
    fn degenerate_and_bad_input_never_panics() {
        let mut app = app(SampleType::U8);
        for r in [[0.0; 4], [5.0, 5.0, 5.0, 5.0], [10.0, 10.0, 0.0, 0.0], [f64::NAN, 0.0, 10.0, 10.0], [f64::INFINITY, 0.0, f64::MAX, 1e300]] {
            app.ui.crop_rect = Some(r);
            for m in [NONE, SHIFT, Modifiers::ALT, SHIFT | Modifiers::ALT] {
                drag(&mut app, &[[5.0, 5.0], [f64::NAN, 3.0], [1e300, -1e300], [6.0, 7.0]], m);
                let _ = cursor(&app, [5.0, 5.0]);
                app.ui.crop_rect = Some(r);
            }
        }
        // Odd angles (the control channel can set any) turn, resize and commit without panicking.
        for a in [f64::NAN, f64::INFINITY, -1e300, 1e-300, 720.0] {
            app.ui.crop_rect = Some([10.0, 10.0, 50.0, 40.0]);
            app.ui.crop_angle = a;
            for pts in [[[30.0, 25.0], [f64::NAN, 1.0]], [[30.0, 25.0], [1e300, 2.0]], [[50.0, 40.0], [60.0, 45.0]], [[90.0, 90.0], [-1e300, 1e300]]] {
                drag(&mut app, &pts, SHIFT);
                let _ = cursor(&app, [5.0, 5.0]);
            }
            assert!(app.ui.crop_angle.is_finite() || !a.is_finite());
            crate::canvas::commit_crop(&mut app);
            app.run("edit.undo", json!({})).ok();
        }
        app.ui.tool_options.crop_ratio = "0:0".into();
        app.ui.crop_rect = Some([0.0, 0.0, 10.0, 10.0]);
        drag(&mut app, &[[10.0, 10.0], [30.0, 30.0]], SHIFT);
        crate::canvas::commit_crop(&mut app);
        // No document: events are swallowed quietly.
        let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        empty.ui.tool = Tool::Crop;
        drag(&mut empty, &[[1.0, 1.0], [9.0, 9.0]], NONE);
        assert!(empty.ui.crop_rect.is_none());
    }

    /// #1792: outside the frame the turn cursor bends towards the corner of the pointer's quadrant
    /// (four variants, as in Photoshop) and turns with the frame; no turn cursor inside or on a handle.
    #[test]
    fn turn_cursor_points_at_the_quadrant_corner() {
        let mut app = app(SampleType::U8);
        app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let near = |d: Option<[f64; 2]>, want: [f64; 2]| d.is_some_and(|d| (d[0] - want[0]).abs() < 1e-9 && (d[1] - want[1]).abs() < 1e-9);
        assert!(near(turn_cursor_dir(&app, [0.0, 0.0]), [h, h]), "top-left: towards the top-left corner");
        assert!(near(turn_cursor_dir(&app, [130.0, 5.0]), [-h, h]), "top-right");
        assert!(near(turn_cursor_dir(&app, [130.0, 90.0]), [-h, -h]), "bottom-right");
        assert!(near(turn_cursor_dir(&app, [5.0, 90.0]), [h, -h]), "bottom-left");
        assert!(near(turn_cursor_dir(&app, [150.0, 30.0]), [-h, h]), "beside the right edge, upper half");
        assert_eq!(turn_cursor_dir(&app, [60.0, 40.0]), None, "inside");
        assert_eq!(turn_cursor_dir(&app, [20.0, 20.0]), None, "on a handle");
        // A frame turned 45° turns the cursor with it: the diagonal becomes horizontal.
        app.ui.crop_angle = 45.0;
        assert!(near(turn_cursor_dir(&app, [0.0, 0.0]), [1.0, 0.0]), "{:?}", turn_cursor_dir(&app, [0.0, 0.0]));
        app.ui.crop_angle = f64::NAN;
        assert!(turn_cursor_dir(&app, [f64::NAN, 0.0]).is_none());
        app.ui.tool = Tool::Brush;
        assert_eq!(turn_cursor_dir(&app, [0.0, 0.0]), None);
    }

    /// A double-click inside the frame commits it, as ↵ does; on a handle or outside it doesn't.
    #[test]
    fn double_click_inside_the_frame_commits() {
        let mut app = app(SampleType::U8);
        app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
        assert!(commits_at(&app, [60.0, 40.0]));
        assert!(!commits_at(&app, [20.0, 20.0]), "on a handle");
        assert!(!commits_at(&app, [150.0, 40.0]), "outside: that turns the frame");
        app.ui.crop_angle = 30.0;
        assert!(commits_at(&app, [60.0, 40.0]), "inside a turned frame");
        assert!(!commits_at(&app, [25.0, 22.0]), "inside the upright frame but outside the turned one");
        app.ui.crop_angle = 0.0;
        app.ui.crop_rect = None;
        assert!(!commits_at(&app, [60.0, 40.0]), "no frame");
        app.ui.crop_rect = Some([f64::NAN, 0.0, 1.0, 1.0]);
        assert!(!commits_at(&app, [0.5, 0.5]));
        app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
        app.ui.tool = Tool::Brush;
        assert!(!commits_at(&app, [60.0, 40.0]));
    }

    #[test]
    fn cursor_follows_the_frame() {
        let mut app = app(SampleType::U8);
        assert_eq!(cursor(&app, [5.0, 5.0]), None, "no frame yet: the default crosshair");
        app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
        assert_eq!(cursor(&app, [20.0, 20.0]), Some(CursorIcon::ResizeNwSe));
        assert_eq!(cursor(&app, [100.0, 20.0]), Some(CursorIcon::ResizeNeSw));
        assert_eq!(cursor(&app, [60.0, 60.0]), Some(CursorIcon::ResizeVertical));
        assert_eq!(cursor(&app, [60.0, 40.0]), Some(CursorIcon::Move));
        assert_eq!(cursor(&app, [150.0, 40.0]), Some(CursorIcon::None), "outside: the drawn turn cursor");
        app.ui.tool = Tool::Brush;
        assert_eq!(cursor(&app, [60.0, 40.0]), None);
    }
}
