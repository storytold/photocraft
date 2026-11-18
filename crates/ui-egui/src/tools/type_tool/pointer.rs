//! The Type tool on the canvas host: a press on shown type edits it, a press elsewhere starts new text
//! on release (a point, a paragraph box dragged out, or text on or in a path).
//!
//! While a session is open the text editor, the text frame and the type transform's move zone take their
//! presses first (`controls.rs`, shown before the tool runs); the editor commits on a background press.
//! A press that reaches the tool with a session open is therefore not the tool's, and the tool does nothing
//! with it.

use craft::behavior::kurbo;
use craft::behavior::{CancelReason, Cursor, CursorIcon, PaintedCursor};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolHostResponse, ToolPath, ToolPointer, ToolRegion};
use craft::ui::primitives::{Draw, PaintShape, StrokeColor, StrokeWidth};
use craft::ui::style::DataColor;

use super::defaults::default_style_params;
use super::editing::begin_edit;
use super::layout::{hit_layer, offset_at};

use super::{LayerId, PLACEHOLDER, PhotocraftApp, Point, TextEdit, active_doc_id, json, session_key, text_layer};
use crate::canvas::document_display::CanvasHost;
use crate::canvas::tools::{FamilyTool, ToolScope};

pub(crate) fn box_shape(app: &PhotocraftApp, id: LayerId) -> Option<(f32, f32, f32, f32)> {
    match app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()))? {
        photocraft_doc::text::TextShape::Box { x, y, width, height } => Some((x, y, width, height)),
        _ => None,
    }
}

/// The Type family's state between passes, kept in `CanvasTools::type_tool`.
#[derive(Debug, Default)]
pub(crate) struct TypeState {
    /// Where the running press began, in document units: the corner of the box a drag makes, or the point of a
    /// click. Set by a press that starts new text, taken by its release.
    pub(super) press: Option<kurbo::Point>,
    /// The press of this pass ended an open session (the editor commits on a background press): it starts no new
    /// text, as in Photoshop. Set by the canvas controls before the tool runs, taken by the press, cleared by `drain`.
    pub(super) began_with_session: bool,
    /// Where a secondary press on the text editor was released and the modifiers held then, taken once by the canvas
    /// body for the canvas menu (the editor's part takes the press, so the host never reports it).
    pub(super) context: Option<(egui::Pos2, Mods)>,
}

impl TypeState {
    pub(crate) fn take_context(&mut self) -> Option<(egui::Pos2, Mods)> {
        self.context.take()
    }
}

/// The Type tool for a pass (`tools::run`), for the four type tools.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut TypeState, _host: CanvasHost) -> FamilyTool<'a> {
    Box::new(TypeTool { scope, state })
}

/// Clears what no gesture is left to take: the press of a gesture that ended some other way, and the flag of a
/// press that never reached the tool.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut TypeState, _response: &ToolHostResponse) {
    state.began_with_session = false;
    if !crate::canvas::tools::in_gesture(app) {
        state.press = None;
    }
}

/// The Type tool as the host runs it: the pass's scope over the app, and the family's state.
pub(crate) struct TypeTool<'a> {
    scope: &'a ToolScope<'a>,
    state: &'a mut TypeState,
}

/// What a Type tool click on a path does, like Photoshop: inside a closed path, area type;
/// on a path's outline, type on the path from that point.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PathClick {
    Area { shape: Option<u64> },
    OnPath { shape: Option<u64>, subpath: usize, start: f64 },
}

/// Arc length (px) along `sub` from its first anchor to segment `seg` at curve parameter `t`.
fn arc_length_to(sub: &photocraft_doc::Subpath, seg: usize, t: f64) -> f64 {
    const PIECES: usize = 32;
    let eval = photocraft_vector::edit::eval;
    let length = |s: &[Point; 4], upto: f64| {
        (1..=PIECES)
            .fold((0.0, eval(s, 0.0)), |(len, prev), j| {
                let q = eval(s, upto * j as f64 / PIECES as f64);
                (len + (q.x - prev.x).hypot(q.y - prev.y), q)
            })
            .0
    };
    let segs = sub.segments();
    let whole: f64 = segs.iter().take(seg).map(|s| length(s, 1.0)).sum();
    whole + segs.get(seg).map_or(0.0, |s| length(s, t.clamp(0.0, 1.0)))
}

/// The path a Type tool click at document `(x, y)` would use: on the outline (within 6 screen px)
/// of the active shape layer's path, else of the work path, type on that path; inside a closed
/// one, area type. The work path's inside counts only while the canvas draws it (selected in the
/// Paths panel, #6551): a hidden Work Path left after Load as Selection must not turn a plain
/// click into area type. None while View › Show › Target Path (or Extras) hides paths.
pub(crate) fn path_target(app: &PhotocraftApp, x: f64, y: f64) -> Option<PathClick> {
    use photocraft_vector::edit::{Hit, hit};
    if !app.ui.view.shows(app.ui.view.show.target_path) {
        return None;
    }
    let shape_path = crate::vector_ui::active_shape_path(app);
    let work = app.session.active().and_then(|s| s.doc.work_path.clone());
    let work_shown = app.ui.selected_path.as_deref() == Some("work");
    // The canvas hover runs this every frame and `hit` samples every segment, so a path with more
    // anchors than this is not a click target (the click and the pointer agree: plain point
    // text). 4096 anchors is far beyond hand-drawn paths and shapes yet keeps a frame cheap, and
    // it is the most `type.fillPath` accepts, so every area click target can be filled.
    const MAX_TARGET_KNOTS: usize = photocraft_text::area::MAX_AREA_KNOTS;
    let small = |path: &photocraft_doc::vector::Path| path.subpaths.iter().map(|s| s.knots.len()).fold(0usize, usize::saturating_add) <= MAX_TARGET_KNOTS;
    let tol = 6.0 / app.point_zoom().max(0.01) as f64;
    // Only the horizontal Type tool starts type on a path: vertical type on a path is not laid
    // out yet.
    let vertical = app.ui.tool.is_vertical_type();
    let outline = |path: &photocraft_doc::vector::Path, shape: Option<u64>| {
        let on_path = |sp: usize, k: usize, t: f64| {
            // Type on a path needs a segment to run along (`type.attachToPath` rejects one anchor).
            let sub = path.subpaths.get(sp).filter(|s| !vertical && s.knots.len() >= 2)?;
            Some(PathClick::OnPath { shape, subpath: sp, start: arc_length_to(sub, k, t) })
        };
        match hit(path, Point::new(x, y), tol, &[]) {
            Some(Hit::Segment([sp, k], t)) => on_path(sp, k, t),
            Some(Hit::Anchor([sp, k])) => on_path(sp, k, 0.0),
            _ => None,
        }
    };
    let inside = |path: &photocraft_doc::vector::Path| photocraft_text::area::AreaShape::new(path, false).is_some_and(|a| a.contains(x as f32, y as f32));
    if let Some((id, p)) = shape_path.as_ref().filter(|(_, p)| small(p)) {
        if let Some(c) = outline(p, Some(*id)) {
            return Some(c);
        }
        if inside(p) {
            return Some(PathClick::Area { shape: Some(*id) });
        }
    }
    let w = work.as_ref().filter(|w| small(w))?;
    if let Some(c) = outline(w, None) {
        return Some(c);
    }
    (work_shown && inside(w)).then_some(PathClick::Area { shape: None })
}

/// What the Type tool's pointer over document `(x, y)` shows (#218): the path click a press there
/// would make, unless the press edits the type layer under it instead (an unshifted press on shown
/// type), where Photoshop keeps the plain I-beam. The type hit is only tested over a path, so a
/// hover elsewhere lays out nothing. Nothing while a type edit is open (a click then commits rather
/// than starting new type). While the button is held (`press` is where it went down) it is what the
/// release makes: nothing once the drag is a text box, else the path click at the press.
pub(crate) fn hover_path_target(app: &mut PhotocraftApp, x: f64, y: f64, shift: bool, press: Option<[f64; 2]>) -> Option<PathClick> {
    if app.ui.text_edit.is_some() {
        return None;
    }
    if let Some(start) = press {
        // The press already passed over editing type (`press_at`), so no type hit test.
        return if makes_box(app, start, [x, y]) { None } else { path_target(app, start[0], start[1]) };
    }
    let target = path_target(app, x, y)?;
    (shift || hit_layer(app, x, y).is_none()).then_some(target)
}

/// Whether a press at `start` released at `end` drags out a paragraph box rather than clicking
/// (at least 4 screen pixels both ways).
fn makes_box(app: &PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> bool {
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let min = 4.0 / app.point_zoom().max(0.01) as f64;
    w >= min && h >= min
}

/// A press at document `(x, y)`: edits the type under it, or answers `Claimed` for a press that makes new text on
/// release. With a session open the press is not the tool's (the editor, the frame or the move zone had it first).
fn press_at(app: &mut PhotocraftApp, ctx: &egui::Context, x: f64, y: f64, mods: Mods) -> ToolAction {
    if app.ui.text_edit.is_some() {
        return ToolAction::Done;
    }
    app.ui.type_insertion_style = None;
    // A ⇧-click starts new text over old, as in Photoshop. A Type Mask click on a text layer's
    // glyphs edits it as the Type tool does (#12075), unless ⇧ is held or the layer is only partly
    // locked: neither is measured, so those keep starting a new mask.
    let hit = hit_layer(app, x, y);
    let partly_locked = |id| app.session.active().map(|st| st.doc.effective_locks(id)).is_some_and(|l| l.any() && !l.all);
    if mods.shift || (app.ui.tool.is_type_mask() && hit.is_some_and(partly_locked)) {
        return ToolAction::Claimed;
    }
    let Some(id) = hit else { return ToolAction::Claimed };
    let _ = app.session.select_layer(id);
    // Lock All (the layer's own or a group's) refuses the edit before it starts, as the engine
    // would refuse the first keystroke; the reference application says so in an alert (#12040).
    if app.session.active().is_some_and(|st| st.doc.effective_locks(id).all) {
        crate::dialogs::alerts::alert(app, tl!("Could not use the type tool because the layer is locked."));
        return ToolAction::Done;
    }
    let key = session_key(app);

    if begin_edit(app, ctx, id, &key).is_err() {
        return ToolAction::Done;
    }
    let off = offset_at(app, id, x, y);
    app.ui.text_edit = Some(TextEdit { layer: id.0, document: active_doc_id(app), caret: off, anchor: off, session: key, created: false, mask: false });
    // The frame commits an edit that is no mask while a Type Mask tool is in hand: edit with
    // the Type tool, as Move's double-click does, and give the mask tool back at the end.
    app.ui.text_edit_return_tool = app.ui.tool.is_type_mask().then_some(app.ui.tool);
    if app.ui.text_edit_return_tool.is_some() {
        let vertical =
            app.session.active().and_then(|st| text_layer(&st.doc, id)).is_some_and(|t| t.orientation == photocraft_doc::text::Orientation::Vertical);
        app.ui.tool = if vertical { crate::state::Tool::VerticalType } else { crate::state::Tool::Type };
    }
    ToolAction::Done
}

/// A release that makes new text: from `start` to `end` in document units. A drag of at least 4 screen pixels both
/// ways makes a paragraph box; anything less is a click, which makes point text, or text on or in the path under it.
fn create_text(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let o = app.ui.tool_options.clone();
    let mask = app.ui.tool.is_type_mask();
    let is_box = makes_box(app, start, end);
    // A click (not a drag) with the Type tool on a path: area type inside a closed path, type
    // on the path on its outline. The Type Mask tools ignore paths.
    let target = if !mask && !is_box { path_target(app, start[0], start[1]) } else { None };
    // Preferences ▸ Type: "Fill new type layers with placeholder text" (on by default). The
    // Horizontal Type Mask gets the Type tool's placeholder, as Photoshop's does (#12072); the
    // vertical mask's placeholder is not measured yet, so it keeps starting empty.
    let placeholder = app.ui.tool != crate::state::Tool::VerticalTypeMask && app.session.prefs().type_.fill_new_type_layers_with_placeholder;
    let text = if placeholder { PLACEHOLDER } else { "" };
    let mut p = default_style_params(app);
    p["text"] = json!(text);
    p["orientation"] = json!(if app.ui.tool.is_vertical_type() { "vertical" } else { "horizontal" });
    if is_box {
        p["box"] = json!([start[0].min(end[0]).round(), start[1].min(end[1]).round(), w.round(), h.round()]);
    } else {
        p["x"] = json!(start[0].round());
        p["y"] = json!(start[1].round());
    }
    let key = session_key(app);
    p["coalesce"] = json!(key);
    if let Ok(v) = app.run("type.create", p)
        && let Some(id) = v.get("layer").and_then(serde_json::Value::as_u64)
    {
        // A failed follow-up leaves point text; say why instead of passing over it.
        let follow_up = match &target {
            Some(PathClick::Area { shape }) => app.run("type.fillPath", json!({"layer": id, "shape": shape, "coalesce": key})),
            Some(PathClick::OnPath { shape, subpath, start }) => {
                app.run("type.attachToPath", json!({"layer": id, "shape": shape, "subpath": subpath, "start": start, "coalesce": key}))
            }
            None => Ok(serde_json::Value::Null),
        };
        if let Err(e) = follow_up {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        if o.type_aa != "sharp" {
            let _ = app.run("type.edit", json!({"layer": id, "antialias": o.type_aa, "coalesce": key}));
        }
        // A Type Mask's text is a hidden working layer: the canvas shows the mask overlay
        // instead, and committing turns it into the selection.
        if mask {
            let _ = app.run("layer.setProps", json!({"layer": id, "visible": false, "coalesce": key}));
        }
        // Like Photoshop: the placeholder is selected, so typing replaces it.
        let n = text.chars().count();
        app.ui.text_edit = Some(TextEdit { layer: id, document: active_doc_id(app), caret: n, anchor: 0, session: key, created: true, mask });
        app.ui.text_edit_return_tool = None;
    }
}

impl CanvasTool for TypeTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        // The press that ended a session (the editor's commit on a background press) only commits.
        if std::mem::take(&mut self.state.began_with_session) {
            return ToolAction::Done;
        }
        let at = space.view.to_doc(pointer.pos);
        let ctx = self.scope.ctx().clone();
        let action = self.scope.with_app(|app| press_at(app, &ctx, at.x, at.y, pointer.mods));
        self.state.press = (action == ToolAction::Claimed).then_some(at);
        action
    }

    /// With an edit open and the type box hidden, another type layer is the tool's over the edit's move reach and
    /// caret: a press there commits the edit (`claimed_regions`). With ⌘ held the box shows, and its own reach rules.
    fn regions(&self) -> Vec<ToolRegion> {
        let mods = craft::ui::runtime::pointer::frame(self.scope.ctx()).mods;
        let regions = self.scope.with_app(|app| {
            let box_shown = app.ui.tool.is_type() && crate::type_transform::whole_box(app, mods);
            if crate::type_transform::active(app) || box_shown {
                return Vec::new();
            }
            super::layout::claimed_regions(app)
        });
        regions.into_iter().map(ToolRegion::polygon).collect()
    }

    fn drag(&mut self, _pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {}

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(start) = self.state.press.take() else { return };
        let Some(space) = cx.space() else { return };
        let end = space.view.to_doc(pointer.pos);
        let ctx = self.scope.ctx().clone();
        self.scope.with_app(|app| {
            create_text(app, [start.x, start.y], [end.x, end.y]);
            // The new session's editor takes the keyboard from the pass it opens, as an edit of existing text does.
            if app.ui.text_edit.is_some() {
                super::editing::focus_editor(&ctx);
            }
        });
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.state.press = None;
        self.state.began_with_session = false;
    }

    /// The box a drag makes, in document axes (#7063): a white line under a black dashed one, so it reads on any
    /// image. Nothing paints without a drag, and a drag too small to make a box makes none.
    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(start), Some(pointer), Some(space)) = (self.state.press, cx.pointer, cx.space()) else { return };
        let end = space.view.to_doc(pointer.pos);
        if !self.scope.with_app(|app| makes_box(app, [start.x, start.y], [end.x, end.y])) {
            return;
        }
        let corner = |x: f64, y: f64| space.view.to_screen(kurbo::Point::new(x, y));
        let ring = [corner(start.x, start.y), corner(end.x, start.y), corner(end.x, end.y), corner(start.x, end.y)];
        draw.stroke(&PaintShape::Polygon(ring.to_vec()), StrokeWidth::Thin, StrokeColor::Data(DataColor::new(egui::Color32::WHITE)));
        let closed = [ring[0], ring[1], ring[2], ring[3], ring[0]];
        draw.dashed(&closed, draw.dashes(), StrokeWidth::Thin, StrokeColor::Data(DataColor::new(egui::Color32::BLACK)));
    }

    /// The I-beam (turned in vertical type) and the path cursor over what a click would start text on or in. The type
    /// transform's box (the move cursor in its move reach just outside the text being typed) and the text frame set
    /// their own cursors over their parts. Type is not among the Other Cursors tools production swaps for the precise
    /// cursor (cw.C9).
    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let (Some(pointer), Some(space)) = (cx.pointer, cx.space()) else { return Cursor::System(CursorIcon::Text) };
        let at = space.view.to_doc(pointer.pos);
        let press = self.state.press.map(|p| [p.x, p.y]);
        self.scope.with_app(|app| match hover_path_target(app, at.x, at.y, cx.mods.shift, press) {
            Some(PathClick::Area { .. }) => Cursor::from(PaintedCursor::type_on(false)),
            Some(PathClick::OnPath { .. }) => Cursor::from(PaintedCursor::type_on(true)),
            None if app.ui.tool.is_vertical_type() => Cursor::System(CursorIcon::VerticalText),
            None => Cursor::System(CursorIcon::Text),
        })
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the type.create command")
    }
}

#[cfg(test)]
#[path = "mask_placeholder_tests.rs"]
mod mask_placeholder_tests;

#[cfg(test)]
#[path = "mask_click_text_layer_tests.rs"]
mod mask_click_text_layer_tests;
