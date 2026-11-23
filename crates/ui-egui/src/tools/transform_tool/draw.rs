//! The Free Transform canvas body: the moving pixels through the session's map, the layers over
//! them, the transformed path outline, and the box or warp controls on top. Show order is paint
//! order, so [`show_in_canvas`] shows the preview first and the controls last.

use craft::behavior::EditEvents;
use craft::behavior::geometry::SnapLine;
use craft::behavior::kurbo::{CubicBez, ParamCurve, Point, Rect};
use craft::draw::ui::pen::{AnchorsShown, BezierOverlay, BezierTool, EditPath, EditSubpath, Knot, PathSelection};
use craft::draw::ui::transform::{ApplyBy, Interpolation, TransformEdit, TransformOverlay, TransformPhase, TransformRequest};
use craft::imaging::ui::selection::PointMap;
use craft::imaging::ui::transform_preview::{PreviewDisplay, PreviewMap, TransformPreview};
use craft::imaging::ui::warp::{CylinderValues, CylinderWarp, WarpEdit, WarpGrid, WarpMesh};
use craft::ui::canvas::RasterDisplay;
use craft::ui::contract::{Bind, Show as _};
use craft::ui::primitives::PaintSlot;
use photocraft_doc::DocId;
use photocraft_geom::warp::WarpStyle;

use super::{OpenPreview, begin, cancel, commit, warp};
use crate::PhotocraftApp;
use crate::state::{FreeTransform, SessionWarp};
use crate::transform_fx::LayerStyleSource;

/// The host key of the transformed path outline in its `EditPath`.
const OUTLINE_KEY: u64 = 1;
/// Points sampled along a curved or bent segment of the path outline.
const OUTLINE_STEPS: usize = 24;

/// What one pass of the controls reported, to apply once the session is no longer borrowed.
#[derive(Default)]
struct Shown {
    edits: Vec<TransformEdit<SessionWarp>>,
    /// The box's snap lines, for the session's canvas tool to hand to the host.
    snapped: Vec<SnapLine>,
    apply: Option<ApplyBy>,
    discard: bool,
    /// The warp grid's events and whether it placed a split this pass.
    mesh: Option<(EditEvents<WarpMesh>, bool)>,
    cylinder: Option<EditEvents<CylinderValues>>,
    /// A warp control has the pointer button down on it.
    busy: bool,
    /// The preview's shapes, for the canvas's picture slot after the document ([`SessionShown::picture`]).
    picture: Vec<egui::Shape>,
}

/// Shows the open Free Transform in the canvas body at `rect`, and applies what its controls
/// report: the preview (moving pixels, covering layers, path outline), then the box or warp
/// controls. The canvas calls it with its controls, before it prepares the document's picture, so
/// the preview's moving pixels come back in [`SessionShown::picture`] for the canvas to add to its
/// picture slot after the document: added here, the document (which hides the moving layer) would
/// paint over them (#15546).
///
/// Every control reads the frame's input once for its own pass, so a second pass of a frame
/// reports no edit again and this applies only what the pass it ran in reported.
///
/// Returns whether the session's box was shown in this pass, even if an edit then ended the session (the Move tool's
/// transform controls share its id and must not open a second box in the same pass), and the box's snap lines, which
/// the session's canvas tool hands to the host in this frame's pass ([`super::canvas_tool`]).
pub fn show_in_canvas(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect, now: f64) -> SessionShown {
    let shown = {
        let (Some(t), Some(pv)) = (&app.ui.transform, &app.transform_preview) else { return SessionShown::default() };
        show_session(app, t, pv, ui, rect)
    };
    let Shown { edits, snapped, apply, discard, mesh, cylinder, busy, picture } = shown;
    let mut need_warp = false;
    if let Some(t) = app.ui.transform.as_mut() {
        for edit in edits.iter().cloned() {
            if matches!(t.session.apply(edit, now), Some(TransformRequest::NeedWarp { .. })) {
                need_warp = true;
            }
        }
    }
    if need_warp {
        begin::enter_warp(app, ui.ctx());
    }
    if let Some((events, split)) = &mesh {
        warp::grid_edit(app, events, *split);
    }
    if let Some(events) = &cylinder {
        warp::cylinder_edit(app, events);
    }
    match apply {
        Some(ApplyBy::Key | ApplyBy::DoubleClick) => commit(app),
        Some(ApplyBy::ClickOutside { at, .. }) => commit::apply_click(app, at),
        _ => {}
    }
    if discard {
        cancel(app);
    }
    commit::auto_commit(app, &edits);
    if let Some(t) = app.ui.transform.as_mut() {
        t.session.settle(now, busy);
    }
    // A pass that ended the session (Cancel, or a commit that finished at once) hands back no picture: its moving
    // pixels would paint over the restored document. A commit still running in the background keeps the preview, and
    // with it the picture.
    let picture = if app.transform_preview.is_some() { picture } else { Vec::new() };
    SessionShown { shown: true, snapped, picture }
}

/// The pass's shows, in paint order.
fn show_session(app: &PhotocraftApp, t: &FreeTransform, pv: &OpenPreview, ui: &mut egui::Ui, rect: egui::Rect) -> Shown {
    let mut out = Shown { picture: show_preview(app, t, pv, ui, rect), ..Shown::default() };
    if pv.committing.is_some() {
        // The moving pixels stay without the box until the commit job lands.
        return out;
    }
    if let Some(outline) = path_outline(t, pv) {
        let selection = PathSelection::default();
        // The outline is display-only: nothing in its output is acted on.
        let _ = BezierOverlay::new("Transform path outline", Bind::given(vec![outline]), &selection)
            .tool(BezierTool::Anchors)
            .anchors(AnchorsShown::Hidden)
            .show_at(ui, rect);
    }
    match t.session.warp() {
        Some(w) => show_warp(t, pv, w, ui, rect, &mut out),
        None => {
            let settings = super::settings(app);
            let shown = TransformOverlay::new("Free Transform", &t.session, &settings).id_salt(super::BOX_ID_SALT).show_at(ui, rect);
            out.edits = shown.edits;
            out.snapped = shown.snapped;
            out.apply = shown.apply;
            out.discard = shown.discard;
        }
    }
    out
}

/// What [`show_in_canvas`] showed in one pass.
#[derive(Default)]
pub struct SessionShown {
    /// A session was shown.
    pub shown: bool,
    /// The snap lines of the box's drag.
    pub snapped: Vec<SnapLine>,
    /// The preview's shapes (the moving pixels, then the layers above them), which the canvas adds to its picture
    /// slot after the document's picture.
    pub picture: Vec<egui::Shape>,
}

/// The moving pixels as the session maps them (warped, skewed or styled), then the layers above
/// them, both in the canvas's picture, under the grid, guides and controls. Nothing is shown for the Frame tool's transform.
fn show_preview(app: &PhotocraftApp, t: &FreeTransform, pv: &OpenPreview, ui: &mut egui::Ui, rect: egui::Rect) -> Vec<egui::Shape> {
    if crate::frame_tool::frame_transform(t) {
        return Vec::new();
    }
    // Painted in place after this mark, then taken out for the picture slot.
    let mark = PaintSlot::reserve(ui.painter());
    let display = preview_display(app, ui.ctx());
    let boxed = t.session.phase() == TransformPhase::Box;
    // The layers that get the styled preview stay PhotoCraft's: a visible layer with a style.
    let source = LayerStyleSource::new(app).filter(|_| boxed && pv.styled.is_some());
    let (map, textures) = match t.session.warp() {
        Some(w) => (PreviewMap::mesh(&w.mesh, t.session.rect()), &pv.textures),
        None => {
            let live = pv.content_aware.as_ref().filter(|_| source.is_none()).and_then(|live| live.shown(t)).map(|(textures, _)| textures);
            (PreviewMap::quad(texture_quad(t, pv)), live.unwrap_or(&pv.textures))
        }
    };
    let canvas = [pv.doc.size.width, pv.doc.size.height];
    let mut preview = TransformPreview::new("Free Transform preview", textures)
        .map(map)
        .opacity(pv.opacity)
        .nearest(t.session.interpolation() == Interpolation::NearestNeighbor)
        .clip(canvas)
        .dragging(t.session.dragging());
    if let Some(display) = display.as_ref() {
        preview = preview.display(display);
    }
    if let (Some(styled), Some(source)) = (&pv.styled, &source) {
        preview = preview.styled(styled, source);
    }
    preview.show_at(ui, rect);
    if let Some(above) = &pv.above
        && let Some(frame) = above.frame()
    {
        RasterDisplay::new("", above).mapped(frame, 1).show_at(ui, rect);
    }
    mark.take_after(ui.ctx())
}

/// What the box draws for the styled layer at `zoom`, as the framework's styled preview builds it: the composite and its
/// plan, or `None` when the box draws the bare pixels (no style, a warp, a part of the layer, the Frame tool). Tests
/// read the pixels here; the preview keeps them as a texture.
#[cfg(test)]
pub(crate) fn styled_composite(app: &PhotocraftApp, zoom: f32) -> Option<(egui::ColorImage, craft::imaging::ui::transform_preview::StylePlan)> {
    use craft::behavior::geometry::Homography;
    use craft::imaging::ui::transform_preview::{PreviewTextures, StyledFrame, StyledSource as _, resample};
    let (t, pv) = (app.ui.transform.as_ref()?, app.transform_preview.as_ref()?);
    if crate::frame_tool::frame_transform(t) || t.session.phase() != TransformPhase::Box || t.session.warp().is_some() {
        return None;
    }
    pv.styled.as_ref()?;
    let source = LayerStyleSource::new(app)?;
    let frame = StyledFrame::new(&pv.textures, texture_quad(t, pv), [pv.doc.size.width, pv.doc.size.height], zoom);
    let plan = craft::imaging::ui::transform_preview::StylePlan::new(
        frame.quad,
        frame.canvas,
        frame.zoom,
        frame.budget.pixels(source.pixel_cost_ns(), false),
        |scale| source.reach(scale),
    )?;
    let level = PreviewTextures::choose(plan.scale as f32, frame.quad_scale, frame.nearest).level;
    let (image, uv) = frame.textures.level_image(level)?;
    let scaled = frame.quad.map(|p| Point::new(p.x * plan.scale, p.y * plan.scale));
    let to_unit = Homography::unit_to_quad(scaled)?.inverse()?;
    let inside = Point::new(scaled.iter().map(|p| p.x / 4.0).sum(), scaled.iter().map(|p| p.y / 4.0).sum());
    let content = resample(&image, uv, &to_unit, inside, plan.content, frame.nearest);
    Some((source.compose(&plan, &content)?, plan))
}

/// The warp grid, or the cylinder editor for a cylinder, with its events for the caller to apply.
fn show_warp(t: &FreeTransform, pv: &OpenPreview, w: &SessionWarp, ui: &mut egui::Ui, rect: egui::Rect, out: &mut Shown) {
    if w.warp.style == WarpStyle::Cylinder {
        if let Some(values) = warp::cylinder_values(&w.warp) {
            let shown = CylinderWarp::new("Warp cylinder", Bind::given(values), &w.mesh).show_at(ui, rect);
            out.busy |= gesture_open(&shown.values.events);
            out.cylinder = Some(shown.values.events);
        }
        return;
    }
    // A preset style fitted as one patch: a split cuts it as it is.
    let one_patch = w.mesh.us.len() == 2 && w.mesh.vs.len() == 2;
    let preset = one_patch && !matches!(w.warp.style, WarpStyle::None | WarpStyle::Custom);
    // No splits on type (#3004).
    let split = pv.split.filter(|_| !w.on_type);
    let mut grid = WarpGrid::new("Warp grid", Bind::given(w.mesh.clone()), t.session.rect());
    if preset {
        grid = grid.preset();
    }
    if w.on_type {
        grid = grid.lines_only();
    }
    if let Some(split) = split {
        grid = grid.split(split);
    }
    let shown = grid.show_at(ui, rect);
    out.busy |= gesture_open(&shown.mesh.events);
    out.mesh = Some((shown.mesh.events, matches!(shown.edit, Some(WarpEdit::Split { .. }))));
}

/// Whether a control's events of this pass leave an edit gesture open: its latest event is a
/// `Begin` or an `Update` (a `Commit` or `Cancel` closes it).
fn gesture_open<T>(events: &EditEvents<T>) -> bool {
    matches!(events.iter().last(), Some(craft::behavior::EditEvent::Begin { .. } | craft::behavior::EditEvent::Update { .. }))
}

/// Where the box puts the moving texture's corners: the box's own, or, when the texture covers
/// more than the frame (the styles of a box over several layers reach past it), where the frame's
/// transform takes the texture's rect.
pub(super) fn texture_quad(t: &FreeTransform, pv: &OpenPreview) -> [Point; 4] {
    let quad = t.session.quad();
    let Some(sv) = pv.several.as_ref().filter(|sv| sv.rect != sv.frame) else { return quad };
    let Some(h) = t.session.homography() else { return quad };
    let r = sv.rect;
    [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| h.apply(Point::new(f64::from(x), f64::from(y))))
}

/// The monitor conversion of the preview, or `None` when the display is the identity. Selection
/// tint and lone mask or channel values are UI diagnostics, not document colours, so those boxes
/// show their texels as they are.
pub(super) fn preview_display(app: &PhotocraftApp, ctx: &egui::Context) -> Option<PreviewDisplay> {
    let (t, pv) = (app.ui.transform.as_ref()?, app.transform_preview.as_ref()?);
    if t.selection || t.target.is_some() || t.path.is_some() {
        return None;
    }
    let output = crate::monitor_status::view_display(app, ctx);
    let (display, _) = crate::canvas::canvas_display(app, &pv.doc, output);
    let display = display.filter(|d| !d.is_identity())?;
    Some(PreviewDisplay::new(display.key, move |image| display_image(image, &display)))
}

/// Premultiplied preview texels -> straight document values -> monitor values. The temporary float
/// buffer is bounded by converting in chunks.
fn display_image(image: &egui::ColorImage, display: &photocraft_engine::display_color::CanvasDisplay) -> egui::ColorImage {
    let mut out = image.clone();
    let convert = |pixels: &mut [egui::Color32]| {
        let px = pixels.iter().map(|p| p.to_srgba_unmultiplied().map(|c| f32::from(c) / 255.0)).collect();
        let buffer = photocraft_compose::Buffer { rect: photocraft_geom::Rect::new(0, 0, pixels.len() as i32, 1), px };
        let converted = display.to_rgba8(&buffer);
        for (p, rgba) in pixels.iter_mut().zip(converted.pixels.as_chunks::<4>().0) {
            *p = egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        out.pixels.par_chunks_mut(4096).for_each(convert);
    }
    #[cfg(target_arch = "wasm32")]
    out.pixels.chunks_mut(4096).for_each(convert);
    out
}

/// The Transform Path outline through the session's map: every flattened point goes through the
/// box's homography (points behind a Distort's horizon are left out) or the warp, and the result
/// is corner-only subpaths for an overlay that shows no anchors. `None` without a path.
fn path_outline(t: &FreeTransform, pv: &OpenPreview) -> Option<EditPath> {
    let path = pv.path.as_ref()?;
    let source = crate::vector_ui::edit_path(path, OUTLINE_KEY);
    let (map, bends): (Box<dyn Fn(Point) -> Option<Point>>, bool) = match t.session.warp() {
        Some(w) => (
            Box::new(move |p| {
                let (x, y) = w.warp.map(p.x, p.y);
                Some(Point::new(x, y))
            }),
            true,
        ),
        None => {
            let h = t.session.homography()?;
            let rect: Rect = t.session.rect();
            (Box::new(move |p| h.apply_in_front(p, rect)), false)
        }
    };
    let subpaths = source.subpaths.iter().flat_map(|sub| mapped_runs(&flatten(sub, bends), sub.closed, &*map)).collect();
    Some(EditPath::new(source.key, subpaths).colored(source.color))
}

/// `sub`'s outline as points: each segment's start, and for a curved segment (or any segment when
/// `bends`, since the map then bends a straight one) points along it. An open subpath ends with
/// its last anchor.
fn flatten(sub: &EditSubpath, bends: bool) -> Vec<Point> {
    let n = sub.knots.len();
    let mut points = Vec::new();
    for (i, a) in sub.knots.iter().enumerate().take(sub.segment_count()) {
        let Some(b) = sub.knots.get((i + 1) % n) else { continue };
        points.push(a.anchor);
        let straight = a.out_ctrl == a.anchor && b.in_ctrl == b.anchor;
        if straight && !bends {
            continue;
        }
        let curve = CubicBez::new(a.anchor, a.out_ctrl, b.in_ctrl, b.anchor);
        points.extend((1..OUTLINE_STEPS).map(|k| curve.eval(k as f64 / OUTLINE_STEPS as f64)));
    }
    if !sub.closed
        && let Some(last) = sub.knots.last()
    {
        points.push(last.anchor);
    }
    points
}

/// `points` through `map` as corner-only subpaths: one closed subpath when every point maps and
/// the original was closed, else one open subpath per run of mapped points (a run of one point
/// draws nothing).
fn mapped_runs(points: &[Point], closed: bool, map: &dyn Fn(Point) -> Option<Point>) -> Vec<EditSubpath> {
    let mapped: Vec<Option<Point>> = points.iter().map(|p| map(*p)).collect();
    let corners = |run: &[Point]| run.iter().map(|p| Knot::corner(*p)).collect::<Vec<_>>();
    if closed && mapped.iter().all(Option::is_some) {
        let all: Vec<Point> = mapped.into_iter().flatten().collect();
        return vec![EditSubpath::new(corners(&all), true)];
    }
    let mut runs = Vec::new();
    let mut run: Vec<Point> = Vec::new();
    for p in mapped {
        match p {
            Some(p) => run.push(p),
            None => runs.push(std::mem::take(&mut run)),
        }
    }
    runs.push(run);
    runs.into_iter().filter(|run| run.len() >= 2).map(|run| EditSubpath::new(corners(&run), false)).collect()
}

/// Applying the open box moves the selection outline with it: `edit.transform` and
/// `edit.transform.warp` map it like the pixels. Not Transform Selection (its ants are hidden), a
/// path, Content-Aware Scale or a type layer's Warp Text, which leave the selection alone.
fn carries_selection(app: &PhotocraftApp, t: &FreeTransform) -> bool {
    !t.selection
        && !crate::frame_tool::frame_transform(t)
        && t.path.is_none()
        && t.content_aware.is_none()
        && !(t.session.warp().is_some() && super::warps_type(app))
}

/// Applying the open box moves the selection outline: Transform Selection's own box, or a box
/// that [carries the selection along](carries_selection). A path, Content-Aware Scale or Warp Text
/// leaves it where it is, so its edges stay snap targets.
pub(crate) fn moves_selection(app: &PhotocraftApp, t: &FreeTransform) -> bool {
    t.selection || carries_selection(app, t)
}

/// A transform of `document` is open that carries its selection along (#5297): the canvas traces
/// the whole outline, not only the part in view where it started, through [`selection_map`].
pub(crate) fn carries_selection_of(app: &PhotocraftApp, document: DocId) -> bool {
    matches!((&app.ui.transform, &app.transform_preview), (Some(t), Some(pv)) if pv.doc.id == document && carries_selection(app, t))
}

/// Where the open box puts the selection of `document` (#5297): the selection follows a move,
/// scale, rotation, skew, distort, perspective or warp while it is made, to where applying it
/// leaves it. `None` when no open transform carries that selection: the canvas shows it where it is.
/// A degenerate box maps nothing, so no outline shows.
pub(crate) fn selection_map(app: &PhotocraftApp, document: DocId) -> Option<PointMap<'_>> {
    if !carries_selection_of(app, document) {
        return None;
    }
    let t = app.ui.transform.as_ref()?;
    if let Some(w) = t.session.warp() {
        return Some(
            PointMap::new(move |p| {
                let (x, y) = w.warp.map(p.x, p.y);
                Some(Point::new(x, y))
            })
            .bends(),
        );
    }
    let rect = t.session.rect();
    Some(match t.session.homography() {
        Some(h) => PointMap::new(move |p| h.apply_in_front(p, rect)).bends(),
        None => PointMap::new(|_| None),
    })
}
