//! The Move tool on the canvas host: a drag moves the selected layers, or the selected pixels
//! (they float until dropped), and a press on empty canvas with Auto-Select draws the layer
//! marquee. The same tool is the ⌘ stand-in of every family that refuses ⌘ (`StandInKey::Command`).
//!
//! The tool's state is [`MoveTool`], kept in `app.canvas_tools.mover`. [`MoveRun`] pairs it with the
//! pass's [`ToolScope`] and is the `CanvasTool` the host runs. `tools::run` takes `app.canvas_tools`
//! for the pass, so code outside a pass reads the state through the queries of [`MoveTool`], and a
//! method of [`MoveRun`] never reads `app.canvas_tools.mover` through the app.

use super::{FamilyTool, ToolScope};
use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;
use crate::state::Tool;
use craft::behavior::geometry::{ConstrainStyle, Directions, MoveGesture};
use craft::behavior::kurbo::{self, Point};
use craft::behavior::{CancelReason, Cursor, CursorIcon};
use craft::keymap::{Chord, Key, Mods};
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::{Draw, PaintShape, StrokeColor, StrokeWidth};
use craft::ui::runtime::env::Label;
use photocraft_engine::float_cmds::Floating;
use serde_json::json;

/// What a press of the Move tool started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// The selected layers follow the pointer.
    Layers,
    /// The selected pixels float and follow the pointer.
    Pixels,
    /// The layer marquee (Auto-Select on empty canvas, or a locked-only hit).
    Marquee,
    /// The press met a locked layer: nothing moves, and the first drag shows the message.
    Refused,
}

/// The gesture in progress, from the press to the release.
struct Run {
    kind: Kind,
    /// The ⇧ lock over the pointer's travel; `None` for a marquee or a refused press.
    gesture: Option<MoveGesture>,
    /// The press, in document units.
    start: [f64; 2],
    /// The moved layers' (or selected pixels') box at the press, in document units.
    box_rect: Option<[f64; 4]>,
    /// The whole-pixel offset so far, as a vector.
    delta: [f64; 2],
    /// The same offset as the release commits it.
    offset: (i32, i32),
    /// The pointer left its press ([`MoveGesture::left_press`]): a press that never does is a click and moves nothing.
    moved: bool,
    /// The refusal message was shown.
    prompted: bool,
    /// The float at the press of a pixels drag (an ⌥-drag drops it and floats a copy).
    floating_before: Option<Floating>,
}

impl Run {
    fn plain(kind: Kind, start: [f64; 2]) -> Self {
        Run { kind, gesture: None, start, box_rect: None, delta: [0.0; 2], offset: (0, 0), moved: false, prompted: false, floating_before: None }
    }
}

/// A whole-layer drag, for the panels' layer transfer (`layer_transfer::{move_drag, arm_move_transfer}`).
pub(crate) struct MoveLayerDrag {
    /// The press, in document space.
    pub start: [f64; 2],
    /// The float captured at the press of a selected-pixels drag; `None` for whole layers.
    pub floating_before: Option<Floating>,
    /// The drag moves the selected pixels (they float with it), not whole layers.
    pub moves_selection: bool,
}

/// The Move tool's state between passes.
#[derive(Default)]
pub(crate) struct MoveTool {
    run: Option<Run>,
}

impl MoveTool {
    /// The live whole-pixel offset of a layer or pixels drag: `Some((0, 0))` from the press, `None` otherwise.
    pub(crate) fn offset(&self) -> Option<(i32, i32)> {
        self.run.as_ref().filter(|r| matches!(r.kind, Kind::Layers | Kind::Pixels)).map(|r| r.offset)
    }

    /// Whether any Move gesture runs (a drag, the layer marquee or a refused press).
    pub(crate) fn dragging(&self) -> bool {
        self.run.is_some()
    }

    /// The moved layers' box at the press and the drag offset so far; `None` outside a layer drag.
    pub(crate) fn dragged_box(&self) -> Option<([f64; 4], [f64; 2])> {
        let r = self.run.as_ref().filter(|r| r.kind == Kind::Layers)?;
        Some((r.box_rect?, r.delta))
    }

    /// The drag under way when it moves layers or selected pixels.
    pub(crate) fn layer_drag(&self) -> Option<MoveLayerDrag> {
        let r = self.run.as_ref().filter(|r| matches!(r.kind, Kind::Layers | Kind::Pixels))?;
        Some(MoveLayerDrag { start: r.start, floating_before: r.floating_before.clone(), moves_selection: r.kind == Kind::Pixels })
    }
}

/// The Move family's tool for a pass (`tools::run`), and the ⌘ stand-in of the families that refuse ⌘.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut MoveTool, _host: CanvasHost) -> FamilyTool<'a> {
    Box::new(MoveRun { scope, state })
}

/// The Move tool as the host runs it: the pass's scope over the app, and the tool's own state.
/// ↩ drops pixels left floating by a move.
const DROP_KEYS: &[Chord] = &[Chord::plain(Key::Enter)];

fn floating(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some())
}

pub(crate) struct MoveRun<'a> {
    scope: &'a ToolScope<'a>,
    state: &'a mut MoveTool,
}

/// Auto-Select's pick moved the active layer: the Layers panel's reveal honours it (`layer_reveal`).
fn note_move_pick(app: &mut PhotocraftApp, active_before: Option<photocraft_doc::LayerId>) {
    let active_after = app.session.active().and_then(|st| st.active_layer);
    if active_after != active_before {
        app.move_pick = active_after.map(|id| (id, app.session.active_layer_changes()));
    }
}

fn covers(sel: &photocraft_raster::Surface, p: [f64; 2]) -> bool {
    let mut v = [0.0f32];
    sel.read_pixel(p[0].floor() as i32, p[1].floor() as i32, &mut v);
    v[0] > 0.0
}

fn inside_selection(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.session.active().and_then(|st| st.doc.selection.as_ref()).is_some_and(|sel| covers(sel, p))
}

fn rect4(r: photocraft_geom::Rect) -> [f64; 4] {
    [f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)]
}

/// The box of what a drag moves, where it is at the press: the selected pixels' outline (the float's
/// piece once cut), else the union of the frames of what moves: a group carries its layers and linked layers go
/// along.
fn moving_box(app: &PhotocraftApp, pixels: bool) -> Option<[f64; 4]> {
    let st = app.session.active()?;
    if pixels {
        let sel = st.doc.selection.as_ref()?;
        let b = match photocraft_engine::float_cmds::floating(st) {
            Some(f) => f.parts.outline().unwrap_or(sel).content_bounds().translate(f.offset.0, f.offset.1),
            None => sel.content_bounds(),
        };
        return (!b.is_empty()).then(|| rect4(b));
    }
    photocraft_engine::layer_multi_cmds::move_targets(&st.doc, st.selected_layers())
        .into_iter()
        .filter_map(|id| crate::snap_ui::tree_rect(&st.doc, id))
        .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

/// The press: Auto-Select's pick, the refusals, then the kind of drag it starts.
fn press_at(app: &mut PhotocraftApp, p: [f64; 2], mods: Mods, style: ConstrainStyle) -> (ToolAction, Option<Run>) {
    // The Cmd stand-in for a painting tool, or the Move tool itself.
    let standing_in = app.ui.tool != Tool::Move;
    // A selection tool's ⌘ press that moves the layer is refused on a locked layer before any pick or marquee
    // (production's `command_moves_layer` check in tool_input.rs `tool_event_with_view`). With the tools on which ⌘ is
    // the Move tool, the press picks the layer under the pointer first and the lock check below refuses only a move.
    if standing_in && super::family(app.ui.tool) == super::Family::Selection && crate::move_lock::blocked(app, true) {
        return (ToolAction::Claimed, Some(Run::plain(Kind::Refused, p)));
    }
    let marquee = |app: &mut PhotocraftApp, add: bool, auto: bool, clear: bool| {
        // PLAN: assumed `move_box::begin(app, p, add, move_auto_select, clear_on_empty)` as today.
        crate::move_box::begin(app, p, add, auto, clear);
        (ToolAction::Claimed, Some(Run::plain(Kind::Marquee, p)))
    };
    let pixels_now = crate::move_ui::moves_selected_pixels_with(app, Tool::Move);
    // A press that doesn't drag the floating piece drops it first.
    if !pixels_now && app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some()) {
        let _ = app.run("select.drop", json!({}));
    }
    // PLAN: `multiple_color_target` lives in `canvas/strokes.rs`; its owner may move it.
    if crate::canvas::strokes::multiple_color_target(app) {
        app.ui.status = "Move and transforms on several color channels are not implemented yet".into();
        return (ToolAction::Done, None);
    }
    // Auto-Select (or ⌘-click while it is off) picks the layer under the pointer first, not when
    // the selected pixels move. A stand-in keeps the ordinary point-pick behaviour.
    if !pixels_now && app.ui.tool_options.move_auto_select != mods.cmd {
        let target = app.ui.tool_options.move_target.clone();
        let auto = !standing_in;
        let mode = if !mods.shift {
            "replace"
        } else if auto {
            "toggle"
        } else {
            "add"
        };
        let probe = app.run("layer.pickAt", json!({"x": p[0], "y": p[1], "target": target, "select": false, "moveAutoSelect": auto}));
        if probe.as_ref().is_ok_and(|v| v["layer"].is_null()) {
            let empty = crate::move_box::empty_at(app, p);
            if empty || auto {
                if !empty {
                    // Pressing locked selected pixels is a move-lock refusal; otherwise a
                    // locked-only hit can start a marquee (#4058).

                    if inside_selection(app, p) && crate::move_lock::blocked(app, standing_in) {
                        return (ToolAction::Claimed, Some(Run::plain(Kind::Refused, p)));
                    }
                    // Shift preserves the base selection for its additive marquee.
                    if !mods.shift {
                        let _ = app.run("select.deselectLayers", json!({}));
                    }
                }
                return marquee(app, mods.shift, auto, auto && !empty);
            }
        } else {
            let locked_hit = !auto
                && probe.as_ref().ok().and_then(|v| v["layer"].as_u64()).is_some_and(|id| {
                    app.session.active().is_some_and(|st| {
                        let id = photocraft_doc::LayerId(id);
                        let locks = st.doc.effective_locks(id);
                        locks.position || locks.all || st.doc.is_background(id)
                    })
                });
            let active_before = app.session.active().and_then(|st| st.active_layer);
            let revealed = crate::layer_reveal::mark(app);
            let _ = app.run("layer.pickAt", json!({"x": p[0], "y": p[1], "target": target, "mode": mode, "moveAutoSelect": auto}));
            note_move_pick(app, active_before);
            crate::layer_reveal::file(app, revealed);
            if locked_hit {
                return marquee(app, mods.shift, false, false);
            }
        }
    }
    // A locked layer: no drag, and the message once the pointer moves.
    if crate::move_lock::blocked(app, standing_in) {
        return (ToolAction::Claimed, Some(Run::plain(Kind::Refused, p)));
    }
    let gesture = MoveGesture::press(&style, Point::new(p[0], p[1]), mods, Directions::Eight);
    // With a selection: cut the selected pixels (⌥ copies them) and drag them as a floating piece,
    // from anywhere.
    if crate::move_ui::moves_selected_pixels_with(app, Tool::Move) {
        let before = app.session.active().and_then(photocraft_engine::float_cmds::floating).cloned();
        if !crate::move_ui::float_selected_for_drag(app, mods.alt) {
            return (ToolAction::Done, None);
        }
        crate::move_mods::begin(app, mods.alt);
        let mut run = Run::plain(Kind::Pixels, p);
        run.gesture = Some(gesture);
        run.box_rect = moving_box(app, true);
        run.floating_before = before;
        return (ToolAction::Claimed, Some(run));
    }
    // PLAN: assumed `move_mods::begin(app, alt)` (tools-w65) latches ⌥ for the duplicate.
    crate::move_mods::begin(app, mods.alt);
    let mut run = Run::plain(Kind::Layers, p);
    run.gesture = Some(gesture);
    run.box_rect = moving_box(app, false);
    (ToolAction::Claimed, Some(run))
}

/// The drag to `pointer`: the ⇧ lock, the snap of the moving box, the ⌥ duplicate and, while the button is down
/// (`readout`), the readout; the release applies its last sample without one.
/// A pass applies only its own pointer sample, so a second pass of a frame moves the offset to
/// the sample it is given and never re-applies an earlier one.
fn follow(run: &mut Run, app: &mut PhotocraftApp, pointer: &ToolPointer, cx: &mut ToolContext<'_>, readout: bool) {
    let Some(space) = cx.space() else { return };
    let Some(gesture) = run.gesture.as_mut() else { return };
    let zoom = space.view.zoom;
    gesture.follow(space.view.to_doc(pointer.pos), pointer.mods, zoom);
    // Inside its press the move is still a click: nothing snaps or moves.
    if !gesture.left_press() {
        return;
    }
    let mut delta = gesture.value().delta;
    if let Some(b) = run.box_rect {
        delta += cx.snap_rect(kurbo::Rect::new(b[0] + delta.x, b[1] + delta.y, b[2] + delta.x, b[3] + delta.y));
    }
    if run.kind == Kind::Layers {
        // PLAN: assumed `move_mods::moved(app, screen_len)` (tools-w65) duplicates once past its threshold.
        crate::move_mods::moved(app, delta.hypot() * zoom);
    }
    let whole = |v: f64| v.round().clamp(-1e7, 1e7) as i32;
    let (dx, dy) = (whole(delta.x), whole(delta.y));
    run.offset = (dx, dy);
    run.delta = [f64::from(dx), f64::from(dy)];
    run.moved = true;
    if readout {
        let x_arrow = if dx < 0 { "←" } else { "→" };
        let y_arrow = if dy < 0 { "↑" } else { "↓" };
        cx.readout(vec![(Label::from(x_arrow), format!("{} px", dx.unsigned_abs())), (Label::from(y_arrow), format!("{} px", dy.unsigned_abs()))]);
    }
}

impl CanvasTool for MoveRun<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        let at = space.view.to_doc(pointer.pos);
        let style = ConstrainStyle::new(f64::from(cx.slop), f64::from(cx.slop));
        // A double-click on type edits it, as in Photoshop (#1132): only the selected Move tool, not a ⌘ stand-in for
        // another tool, and not while a transform box is open (its double-click commits).
        if pointer.clicks >= 2 {
            let edited = self.scope.with_app(|app| {
                if app.ui.tool != Tool::Move || app.ui.transform.is_some() {
                    return false;
                }
                let ctx = app.ctx().clone();
                crate::type_tool::edit_at(app, &ctx, at.x, at.y)
            });
            if edited {
                self.state.run = None;
                return ToolAction::Done;
            }
        }
        let (action, run) = self.scope.with_app(|app| press_at(app, [at.x, at.y], pointer.mods, style));
        self.state.run = run;
        action
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(run) = self.state.run.as_mut() else { return };
        let Some(space) = cx.space() else { return };
        let at = space.view.to_doc(pointer.pos);
        match run.kind {
            Kind::Marquee => {
                self.scope.with_app(|app| crate::move_box::drag_to(app, [at.x, at.y]));
            }
            Kind::Refused => {
                if !std::mem::replace(&mut run.prompted, true) {
                    self.scope.with_app(crate::move_lock::prompt);
                }
            }
            Kind::Layers | Kind::Pixels => self.scope.with_app(|app| follow(run, app, pointer, cx, true)),
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(mut run) = self.state.run.take() else { return };
        let Some(space) = cx.space() else { return };
        let at = space.view.to_doc(pointer.pos);
        self.scope.with_app(|app| match run.kind {
            Kind::Marquee => {
                crate::move_box::release(app, [at.x, at.y]);
            }
            Kind::Refused => {}
            Kind::Layers | Kind::Pixels => {
                // A click (released without leaving its press) never moves: `follow` snaps nothing inside the press.
                follow(&mut run, app, pointer, cx, false);
                let (dx, dy) = run.offset;
                if run.kind == Kind::Layers {
                    if run.moved {
                        crate::move_ui::finish(app, f64::from(dx), f64::from(dy));
                    } else {
                        app.move_preview = None;
                    }
                } else if !run.moved {
                    // The piece never moved: Undo isn't spent putting it back.
                    let _ = app.run("select.drop", json!({}));
                } else if (dx, dy) != (0, 0)
                    && let Err(e) = app.run("select.float", json!({"dx": dx, "dy": dy}))
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
                if run.kind == Kind::Pixels {
                    // The floating piece shows from the document now, also when the drag snapped back to where it
                    // began: the drag's preview is spent.
                    app.move_preview = None;
                }
                crate::move_mods::finish(app);
            }
        });
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.state.run = None;
        self.scope.with_app(|app| {
            crate::move_box::cancel(app);
            crate::move_mods::abandon(app);
            app.move_preview = None;
        });
    }

    fn claims(&self) -> ToolClaims {
        if self.state.run.is_none() && self.scope.with_app(|app| floating(app)) { ToolClaims::NONE.keys(DROP_KEYS) } else { ToolClaims::NONE }
    }

    fn key(&mut self, key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        let enter = match key {
            ToolKey::Submit => true,
            ToolKey::Kept(chord) => *chord == DROP_KEYS[0],
            _ => false,
        };
        enter
            && self.state.run.is_none()
            && self.scope.with_app(|app| {
                if !floating(app) {
                    return false;
                }
                if let Err(e) = app.run("select.drop", json!({})) {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
                true
            })
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let Some(run) = self.state.run.as_ref().filter(|r| r.moved && matches!(r.kind, Kind::Layers | Kind::Pixels)) else { return };
        // The layers follow the pointer themselves; the arrow only when they can't.
        if self.scope.with_app(|app| crate::move_ui::showing(app)) {
            return;
        }
        let Some(space) = cx.space() else { return };
        let Some(style) = cx.style() else { return };
        let from = space.view.to_screen(Point::new(run.start[0], run.start[1]));
        let to = space.view.to_screen(Point::new(run.start[0] + run.delta[0], run.start[1] + run.delta[1]));
        let travel = to - from;
        if travel.length() < 1.0 {
            return;
        }
        let dir = travel.normalized();
        let side = egui::vec2(-dir.y, dir.x);
        let accent = StrokeColor::Theme(style.color(style.tokens.color.overlay.line));
        draw.stroke(&PaintShape::Path(vec![from, to]), StrokeWidth::Medium, accent);
        draw.stroke(&PaintShape::Path(vec![to - dir * 10.0 + side * 5.0, to, to - dir * 10.0 - side * 5.0]), StrokeWidth::Medium, accent);
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let alt = cx.mods.alt;
        if self.scope.with_app(|app| crate::move_mods::copying(app, alt)) { Cursor::System(CursorIcon::Copy) } else { Cursor::System(CursorIcon::Default) }
    }

    /// A layer or pixel move takes every move from the press, so a move of a few pixels moves; its click is decided by
    /// [`MoveGesture::left_press`] alone.
    fn takes_every_move(&self) -> bool {
        self.state.run.as_ref().is_some_and(|run| matches!(run.kind, Kind::Layers | Kind::Pixels))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Move Layer command")
    }
}
