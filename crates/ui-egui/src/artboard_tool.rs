//! Artboard tool (Move group, V / ⇧V cycles with the Move tool): drag on an empty spot to draw a
//! new artboard, click a board to select it, drag it to move it with its contents, and drag the
//! selected board's edge and corner handles to resize it (⇧ on a corner keeps its proportions).
//! The "+" buttons beside the selected board's sides add a board of the same size there (⌥: a copy
//! with its contents). The options bar sets the selected board's size (preset, W/H,
//! portrait/landscape) and adds a board of the same size beside the last one. Every change is one
//! engine command (`layer.new.artboard`, `layer.artboard.set`, `layer.artboard.addBeside`), so each
//! gesture is one undo step; this module only turns pointer gestures into them.

use egui::{CursorIcon, Pos2, Rect, Stroke, vec2};
use photocraft_doc::{Document, Layer, LayerId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::crop_ui::{Hit, hit};
use crate::state::Tool;
use crate::theme::Tokens;

/// How far from an edge (screen points) a press still grabs its handle.
const HANDLE_PX: f64 = 6.0;
/// A press that moves less than this (screen points) is a click, not a drawn board.
const CLICK_PX: f64 = 3.0;
/// How far the "+" buttons sit outside the selected board's edges (screen points, to their centre).
const PLUS_OFFSET_PX: f64 = 18.0;
/// The "+" buttons' radius (screen points).
const PLUS_RADIUS_PX: f64 = 8.0;
/// The sides the "+" buttons add a board on, as `layer.artboard.addBeside` names them, with the
/// direction from the board's centre.
const SIDES: [(&str, [f64; 2]); 4] = [("top", [0.0, -1.0]), ("right", [1.0, 0.0]), ("bottom", [0.0, 1.0]), ("left", [-1.0, 0.0])];

/// Artboard tool state (serde: readable through the control channel).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtboardToolUi {
    #[serde(skip)]
    pub drag: Option<ArtboardDrag>,
}

/// A gesture in progress. Board rects are `[x0, y0, x1, y1]` in document pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArtboardDrag {
    /// Drawing a new board from `start`.
    New { start: [f64; 2], cur: [f64; 2] },
    /// Moving board `id`, whose rect was `rect` at the press.
    Move { id: LayerId, rect: [i64; 4], start: [f64; 2], cur: [f64; 2] },
    /// Dragging handle (`hx`, `hy`) of board `id`: −1 the left/top edge, 1 the right/bottom, 0 neither.
    /// `keep`: ⇧ is down, so a corner keeps the board's proportions.
    Resize { id: LayerId, rect: [i64; 4], hx: i8, hy: i8, start: [f64; 2], cur: [f64; 2], keep: bool },
}

fn edges(r: photocraft_geom::Rect) -> [i64; 4] {
    [i64::from(r.x0), i64::from(r.y0), i64::from(r.x1), i64::from(r.y1)]
}

fn as_f64(r: [i64; 4]) -> [f64; 4] {
    r.map(|v| v as f64)
}

/// `[x, y, width, height]` for the engine's `rect` param.
fn xywh(r: [i64; 4]) -> [i64; 4] {
    [r[0], r[1], r[2].saturating_sub(r[0]), r[3].saturating_sub(r[1])]
}

/// The selected board: the active layer's artboard, with its edges.
fn active_board(app: &PhotocraftApp) -> Option<(LayerId, [i64; 4])> {
    let st = app.session.active()?;
    let id = st.doc.artboard_of(st.active_layer?)?;
    Some((id, edges(st.doc.layer(id).and_then(Layer::artboard)?.rect)))
}

/// The topmost board under a document point.
fn board_at(doc: &Document, p: [f64; 2]) -> Option<(LayerId, [i64; 4])> {
    let inside = |r: [f64; 4]| p[0] >= r[0] && p[0] < r[2] && p[1] >= r[1] && p[1] < r[3];
    doc.artboards().iter().rev().map(|b| (b.0, edges(b.2.rect))).find(|b| inside(as_f64(b.1)))
}

fn tolerance(app: &PhotocraftApp) -> f64 {
    HANDLE_PX / f64::from(app.point_zoom()).max(0.01)
}

/// The selected board's handle under a document point.
fn handle_at(app: &PhotocraftApp, p: [f64; 2]) -> Option<(LayerId, [i64; 4], i8, i8)> {
    let (id, r) = active_board(app)?;
    match hit(as_f64(r), p, tolerance(app)) {
        Hit::Handle(hx, hy) => Some((id, r, hx, hy)),
        _ => None,
    }
}

/// Each side's name and "+" button centre.
type PlusButtons = [(&'static str, [f64; 2]); 4];

/// The "+" buttons of the selected board: its id and each side's name and button centre (document
/// pixels).
fn plus_buttons(app: &PhotocraftApp) -> Option<(LayerId, PlusButtons)> {
    let (id, r) = active_board(app)?;
    let r = as_f64(r);
    let off = PLUS_OFFSET_PX / f64::from(app.point_zoom()).max(0.01);
    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
    let (hw, hh) = ((r[2] - r[0]) / 2.0 + off, (r[3] - r[1]) / 2.0 + off);
    Some((id, SIDES.map(|(side, [dx, dy])| (side, [cx + dx * hw, cy + dy * hh]))))
}

/// The side of the "+" button under a document point.
fn plus_at(app: &PhotocraftApp, p: [f64; 2]) -> Option<(LayerId, &'static str)> {
    let (id, buttons) = plus_buttons(app)?;
    let r = PLUS_RADIUS_PX / f64::from(app.point_zoom()).max(0.01);
    buttons.iter().find(|(_, c)| (p[0] - c[0]).hypot(p[1] - c[1]) <= r).map(|(side, _)| (id, *side))
}

/// `id` and every layer inside it: a board never snaps to itself or its own contents.
fn board_and_contents(doc: &Document, id: LayerId) -> Vec<LayerId> {
    let mut out = vec![id];
    let mut stack: Vec<&Layer> = doc.layer(id).and_then(Layer::children).map(|c| c.iter().collect()).unwrap_or_default();
    while let Some(l) = stack.pop() {
        out.push(l.id);
        stack.extend(l.children().into_iter().flatten());
    }
    out
}

/// What a press at `p` snaps (`snap_ui`): the dragged handle of the selected board, a board moved
/// from inside (its edges and centre), or the corner of a new board; nothing on a "+" button.
pub(crate) fn snap_gesture(app: &PhotocraftApp, p: [f64; 2]) -> Option<(crate::snap_ui::Gesture, Vec<LayerId>)> {
    use crate::snap_ui::Gesture;
    if plus_at(app, p).is_some() {
        return None;
    }
    let doc = &app.session.active()?.doc;
    if let Some((id, r, hx, hy)) = handle_at(app, p) {
        let at = |h: i8, lo: i64, hi: i64| match h {
            -1 => lo as f64,
            1 => hi as f64,
            _ => (lo as f64 + hi as f64) / 2.0,
        };
        let gesture = Gesture::Handle { point: [at(hx, r[0], r[2]), at(hy, r[1], r[3])], axes: [hx != 0, hy != 0] };
        // Resizing leaves the contents in place: the edge snaps to them too, to fit the board to
        // the artwork.
        return Some((gesture, vec![id]));
    }
    if let Some((id, r)) = board_at(doc, p) {
        return Some((Gesture::Move { rect: as_f64(r) }, board_and_contents(doc, id)));
    }
    Some((Gesture::Point, Vec::new()))
}

/// Screen points above a board's top edge that count as its name strip (the Move tool drags the
/// board from there, as from its outline).
const LABEL_PX: f64 = 18.0;

/// The topmost board whose outline (or name strip just above it) is under `p`: what the Move tool
/// drags as a board instead of the image inside it.
fn board_edge_at(app: &PhotocraftApp, p: [f64; 2]) -> Option<(LayerId, [i64; 4])> {
    let st = app.session.active()?;
    let tol = tolerance(app);
    let label = LABEL_PX / f64::from(app.point_zoom()).max(0.01);
    st.doc.artboards().iter().rev().map(|b| (b.0, edges(b.2.rect))).find(|(_, r)| {
        let r = as_f64(*r);
        let near_x = p[0] >= r[0] - tol && p[0] <= r[2] + tol;
        let near_y = p[1] >= r[1] - tol && p[1] <= r[3] + tol;
        let on_edge =
            (near_y && ((p[0] - r[0]).abs() <= tol || (p[0] - r[2]).abs() <= tol)) || (near_x && ((p[1] - r[1]).abs() <= tol || (p[1] - r[3]).abs() <= tol));
        let on_label = p[0] >= r[0] && p[0] <= r[2] && p[1] < r[1] && p[1] >= r[1] - label;
        on_edge || on_label
    })
}

/// The Move tool over a board's outline or name strip: its move cursor.
pub fn move_tool_cursor(app: &PhotocraftApp, p: [f64; 2]) -> Option<CursorIcon> {
    (app.ui.tool == Tool::Move && app.ui.transform.is_none() && crate::rulers::guide_at(app, p[0], p[1]).is_none())
        .then(|| board_edge_at(app, p))
        .flatten()
        .map(|_| CursorIcon::Move)
}

/// What a Move tool press at `p` snaps when it grabs a board by its edge (`snap_ui`).
pub(crate) fn move_tool_snap(app: &PhotocraftApp, p: [f64; 2]) -> Option<(crate::snap_ui::Gesture, Vec<LayerId>)> {
    move_tool_cursor(app, p)?;
    let (id, r) = board_edge_at(app, p)?;
    Some((crate::snap_ui::Gesture::Move { rect: as_f64(r) }, board_and_contents(&app.session.active()?.doc, id)))
}

/// The board drawn from `a` to `b`, edges on whole pixels.
fn drawn(a: [f64; 2], b: [f64; 2]) -> [i64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])].map(|v| v.round() as i64)
}

/// `d` rounded to whole pixels.
fn offset(start: [f64; 2], cur: [f64; 2]) -> [i64; 2] {
    [(cur[0] - start[0]).round() as i64, (cur[1] - start[1]).round() as i64]
}

/// How far a gesture's whole-pixel rounding moves its edges from the pointer, per axis.
fn rounding(d: &ArtboardDrag) -> [f64; 2] {
    match *d {
        ArtboardDrag::New { cur, .. } => [cur[0].round() - cur[0], cur[1].round() - cur[1]],
        ArtboardDrag::Move { start, cur, .. } | ArtboardDrag::Resize { start, cur, .. } => {
            let o = offset(start, cur);
            [o[0] as f64 - (cur[0] - start[0]), o[1] as f64 - (cur[1] - start[1])]
        }
    }
}

/// `r` with handle (`hx`, `hy`) dragged by `d`: an edge dragged past the opposite one flips the
/// board, which never gets narrower than one pixel.
fn resized(r: [i64; 4], hx: i8, hy: i8, d: [i64; 2]) -> [i64; 4] {
    let mut e = r;
    for (h, lo, hi, dv) in [(hx, 0, 2, d[0]), (hy, 1, 3, d[1])] {
        match h {
            -1 => e[lo] = e[lo].saturating_add(dv),
            1 => e[hi] = e[hi].saturating_add(dv),
            _ => {}
        }
        let (a, b) = (e[lo].min(e[hi]), e[lo].max(e[hi]));
        (e[lo], e[hi]) = (a, b.max(a.saturating_add(1)));
    }
    e
}

/// `r` with corner (`hx`, `hy`) dragged by `d`, keeping its proportions (⇧): the opposite corner
/// stays, and the board scales by the larger of the two axes' factors.
fn resized_keep(r: [i64; 4], hx: i8, hy: i8, d: [i64; 2]) -> [i64; 4] {
    let (w, h) = ((r[2] - r[0]).max(1) as f64, (r[3] - r[1]).max(1) as f64);
    // The fixed corner, and the dragged one's offset from it.
    let (ax, ay) = (if hx > 0 { r[0] } else { r[2] }, if hy > 0 { r[1] } else { r[3] });
    let (cx, cy) = (if hx > 0 { r[2] } else { r[0] }.saturating_add(d[0]), if hy > 0 { r[3] } else { r[1] }.saturating_add(d[1]));
    let (dw, dh) = (cx.saturating_sub(ax) as f64, cy.saturating_sub(ay) as f64);
    let k = (dw.abs() / w).max(dh.abs() / h);
    let (nw, nh) = ((w * k).round().max(1.0) as i64, (h * k).round().max(1.0) as i64);
    let span = |a: i64, len: i64, toward: f64| if toward < 0.0 { [a.saturating_sub(len), a] } else { [a, a.saturating_add(len)] };
    let ([x0, x1], [y0, y1]) = (span(ax, nw, dw), span(ay, nh, dh));
    [x0, y0, x1, y1]
}

/// The board a gesture would leave, for the preview.
fn preview(d: &ArtboardDrag) -> [i64; 4] {
    match *d {
        ArtboardDrag::New { start, cur } => drawn(start, cur),
        ArtboardDrag::Move { rect, start, cur, .. } => {
            let o = offset(start, cur);
            [rect[0].saturating_add(o[0]), rect[1].saturating_add(o[1]), rect[2].saturating_add(o[0]), rect[3].saturating_add(o[1])]
        }
        ArtboardDrag::Resize { rect, hx, hy, start, cur, keep: true, .. } if hx != 0 && hy != 0 => resized_keep(rect, hx, hy, offset(start, cur)),
        ArtboardDrag::Resize { rect, hx, hy, start, cur, .. } => resized(rect, hx, hy, offset(start, cur)),
    }
}

fn report(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Moves a gesture's pointer to `p`; ⇧ (held or released mid-drag) decides whether a corner keeps
/// the board's proportions.
fn follow(d: &mut ArtboardDrag, p: [f64; 2], mods: egui::Modifiers) {
    match d {
        ArtboardDrag::New { cur, .. } | ArtboardDrag::Move { cur, .. } => *cur = p,
        ArtboardDrag::Resize { cur, keep, .. } => (*cur, *keep) = (p, mods.shift),
    }
}

/// The Move tool drags a board by its outline or name strip, as Photoshop does; anywhere else it
/// moves the layers as usual. Called before the Move tool's own handlers (its transform controls
/// and layer moves would otherwise take the drag), right after snapping. Returns true when the
/// event was a board drag's.
pub fn move_tool_pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if app.ui.tool != Tool::Move {
        return false;
    }
    if let ToolEvent::Down { x, y, .. } = ev {
        let Some((id, rect)) = (!mods.command && !mods.alt).then(|| move_tool_cursor(app, [x, y]).and_then(|_| board_edge_at(app, [x, y]))).flatten() else {
            // A drag whose release never arrived (the pointer let go outside the window) mustn't
            // take over this unrelated press.
            app.ui.artboard_tool.drag = None;
            return false;
        };
        if app.session.active().and_then(|st| st.active_layer) != Some(id) {
            let r = app.run("layer.select", json!({"layer": id.0}));
            report(app, r);
        }
        app.ui.artboard_tool.drag = Some(ArtboardDrag::Move { id, rect, start: [x, y], cur: [x, y] });
        return true;
    }
    if !matches!(app.ui.artboard_tool.drag, Some(ArtboardDrag::Move { .. })) {
        return false;
    }
    gesture(app, ev, mods);
    true
}

/// Pointer input for the Artboard tool. Returns true when the event was its own.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    // ⌘ is the Move tool (`hold_keys::cmd_moves`): a press with it held is not this tool's,
    // so it drags a guide or the layer (#6288). A drag already under way keeps its events.
    if app.ui.tool != Tool::Artboard || (app.ui.artboard_tool.drag.is_none() && crate::canvas::event_tool(app, mods) != Tool::Artboard) {
        return false;
    }
    gesture(app, ev, mods);
    true
}

/// The Artboard tool's press, drag and release (the Move tool's board drags share the drag and
/// release).
fn gesture(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let p = [x, y];
            // A "+" button: a board the size of the selected one on that side (⌥: a copy).
            if let Some((id, side)) = plus_at(app, p) {
                app.ui.artboard_tool.drag = None;
                let r = app.run("layer.artboard.addBeside", json!({"layer": id.0, "side": side, "duplicate": mods.alt}));
                report(app, r);
                return;
            }
            app.ui.artboard_tool.drag = Some(if let Some((id, rect, hx, hy)) = handle_at(app, p) {
                ArtboardDrag::Resize { id, rect, hx, hy, start: p, cur: p, keep: mods.shift }
            } else if let Some((id, rect)) = board_at(&doc, p) {
                if app.session.active().and_then(|st| st.active_layer) != Some(id) {
                    let r = app.run("layer.select", json!({"layer": id.0}));
                    report(app, r);
                }
                ArtboardDrag::Move { id, rect, start: p, cur: p }
            } else {
                ArtboardDrag::New { start: p, cur: p }
            });
        }
        ToolEvent::Move { x, y, .. } => {
            if let Some(d) = &mut app.ui.artboard_tool.drag {
                follow(d, [x, y], mods);
                // Snapped to a guide at 22.5, the board lands on a whole pixel: the snap lines
                // follow it there (by the drag's own rounding, so a centre on a half pixel stays).
                let e = rounding(d);
                for l in &mut app.prefs_rt.snap_lines {
                    l.pos += e[usize::from(!l.vertical)];
                }
            }
        }
        ToolEvent::Up { x, y } => {
            let Some(mut d) = app.ui.artboard_tool.drag.take() else { return };
            follow(&mut d, [x, y], mods);
            let r = preview(&d);
            let result = match d {
                ArtboardDrag::New { start, cur } => {
                    let screen = (cur[0] - start[0]).hypot(cur[1] - start[1]) * f64::from(app.point_zoom());
                    if screen < CLICK_PX || r[2] <= r[0] || r[3] <= r[1] {
                        return;
                    }
                    // Like Photoshop, the new board takes in the layers drawn around.
                    app.run("layer.new.artboard", json!({"rect": xywh(r), "adopt": true}))
                }
                ArtboardDrag::Move { id, rect, .. } if r != rect => app.run("layer.artboard.set", json!({"layer": id.0, "x": r[0], "y": r[1]})),
                // Resizing leaves the contents where they are, as in Photoshop.
                ArtboardDrag::Resize { id, rect, .. } if r != rect => {
                    app.run("layer.artboard.set", json!({"layer": id.0, "rect": xywh(r), "moveContents": false}))
                }
                _ => return,
            };
            report(app, result);
        }
    }
}

/// The resize pointer for handle (`hx`, `hy`) as the view shows it, or none away from the
/// selected board's handles.
pub fn cursor(app: &PhotocraftApp, xf: &ViewXform, p: [f64; 2]) -> Option<CursorIcon> {
    let (hx, hy) = match app.ui.artboard_tool.drag {
        Some(ArtboardDrag::Resize { hx, hy, .. }) => (hx, hy),
        Some(_) => return None,
        None if plus_at(app, p).is_some() => return Some(CursorIcon::PointingHand),
        None => handle_at(app, p).map(|h| (h.2, h.3))?,
    };
    // (hx, hy) is the handle's outward direction: -1, 0 or 1 on each axis.
    Some(crate::tool_cursor::resize([f64::from(hx), f64::from(hy)], xf))
}

/// The selected board's handles, its "+" buttons, and the board a gesture in progress would leave.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let moving_with_move_tool = app.ui.tool == Tool::Move && app.ui.artboard_tool.drag.is_some();
    if app.ui.tool != Tool::Artboard && !moving_with_move_tool {
        return;
    }
    let t = Tokens::get(painter.ctx());
    let pt = |x: i64, y: i64| xf.to_screen(x as f32, y as f32);
    let outline = |r: [i64; 4]| -> Vec<Pos2> { vec![pt(r[0], r[1]), pt(r[2], r[1]), pt(r[2], r[3]), pt(r[0], r[3]), pt(r[0], r[1])] };
    let drag = app.ui.artboard_tool.drag;
    if let Some(d) = drag.as_ref() {
        let r = preview(d);
        painter.add(egui::Shape::line(outline(r), Stroke::new(1.5, t.accent)));
        // The size while drawing or resizing, the position while moving, beside the pointer.
        let (labels, values, cur) = match *d {
            ArtboardDrag::Move { cur, .. } => (["X:", "Y:"], crate::canvas::size_readout(app, [0.0, 0.0, r[0] as f64, r[1] as f64]), cur),
            ArtboardDrag::New { cur, .. } | ArtboardDrag::Resize { cur, .. } => (["W:", "H:"], crate::canvas::size_readout(app, as_f64(r)), cur),
        };
        let at = xf.to_screen(cur[0] as f32, cur[1] as f32);
        crate::canvas::draw_value_readout(app, painter.ctx(), "artboard-readout", at, labels, values);
    }
    if moving_with_move_tool {
        return;
    }
    let Some((_, r)) = active_board(app).filter(|_| !matches!(drag, Some(ArtboardDrag::New { .. }))) else { return };
    if drag.is_none()
        && let Some((_, buttons)) = plus_buttons(app)
    {
        let rad = PLUS_RADIUS_PX as f32;
        for (_, c) in buttons {
            let c = xf.to_screen(c[0] as f32, c[1] as f32);
            painter.circle(c, rad, t.card, Stroke::new(1.0, t.accent));
            let arm = rad * 0.5;
            let s = Stroke::new(1.5, t.accent);
            painter.line_segment([c - vec2(arm, 0.0), c + vec2(arm, 0.0)], s);
            painter.line_segment([c - vec2(0.0, arm), c + vec2(0.0, arm)], s);
        }
    }
    let r = drag.as_ref().map_or(r, preview);
    let (mx, my) = (r[0].saturating_add(r[2]) / 2, r[1].saturating_add(r[3]) / 2);
    for (x, y) in [(r[0], r[1]), (mx, r[1]), (r[2], r[1]), (r[2], my), (r[2], r[3]), (mx, r[3]), (r[0], r[3]), (r[0], my)] {
        let h = Rect::from_center_size(pt(x, y), vec2(7.0, 7.0));
        painter.rect_filled(h, 0.0, egui::Color32::WHITE);
        painter.rect_stroke(h, 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
}

/// The Artboard tool's options bar: the selected board's size preset, W/H, Make Portrait / Make
/// Landscape, and Add New Artboard.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::Artboard {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    // While a board is moved or resized, its fields follow the gesture.
    let board = active_board(app).map(|(id, r)| match app.ui.artboard_tool.drag {
        Some(d @ (ArtboardDrag::Move { id: did, .. } | ArtboardDrag::Resize { id: did, .. })) if did == id => (id, preview(&d)),
        _ => (id, r),
    });
    let preset = board.and_then(|(id, _)| app.session.active()?.doc.layer(id)?.artboard().map(|a| a.preset.clone()));
    let mut edit: Option<Value> = None;
    ui.add_enabled_ui(board.is_some(), |ui| {
        let (id, r) = board.unwrap_or((LayerId(0), [0, 0, 0, 0]));
        ui.label(egui::RichText::new(tl!("Size")).color(t.text_dim));
        let mut preset = preset.unwrap_or_default();
        let mut opts: Vec<(String, &str)> = vec![(String::new(), tl!("Custom"))];
        opts.extend(photocraft_engine::artboard_cmds::PRESETS.iter().map(|(n, _, _)| (n.to_string(), *n)));
        if crate::widgets::dropdown(ui, "artboard-tool-preset", tl!("Preset"), &mut preset, &opts, 150.0) && !preset.is_empty() {
            edit = Some(json!({"layer": id.0, "preset": preset}));
        }
        let (mut w, mut h) = ((r[2] - r[0]) as f32, (r[3] - r[1]) as f32);
        ui.label(egui::RichText::new(tl!("W")).color(t.text_dim));
        let cw = crate::widgets::value_field(ui, &mut w, 1.0..=300_000.0, "px", 64.0).accessible_name(tl!("Width")).changed();
        ui.label(egui::RichText::new(tl!("H")).color(t.text_dim));
        let ch = crate::widgets::value_field(ui, &mut h, 1.0..=300_000.0, "px", 64.0).accessible_name(tl!("Height")).changed();
        if cw || ch {
            edit = Some(json!({"layer": id.0, "width": w.round(), "height": h.round(), "coalesce": format!("artboard-tool-wh-{}", id.0)}));
        }
        if crate::widgets::secondary_button(ui, tl!("Make Portrait"), 0.0).clicked() && w > h {
            edit = Some(json!({"layer": id.0, "width": h.round(), "height": w.round()}));
        }
        if crate::widgets::secondary_button(ui, tl!("Make Landscape"), 0.0).clicked() && h > w {
            edit = Some(json!({"layer": id.0, "width": h.round(), "height": w.round()}));
        }
    });
    if let Some(p) = edit {
        let r = app.run("layer.artboard.set", p);
        report(app, r);
    }
    crate::widgets::vline(ui, 22.0);
    let has_doc = app.session.active().is_some();
    if ui.add_enabled_ui(has_doc, |ui| crate::widgets::secondary_button(ui, tl!("Add New Artboard"), 0.0)).inner.clicked() {
        // A board the size of the selected one, right of the rightmost board (the canvas size
        // without a selected board).
        let p = board.map_or(json!({}), |(_, r)| json!({"width": r[2] - r[0], "height": r[3] - r[1]}));
        let r = app.run("layer.new.artboard", p);
        report(app, r);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::tool_event;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.ui.tool = Tool::Artboard;
        app
    }

    fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
        let m = egui::Modifiers::NONE;
        tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
    }

    fn boards(app: &PhotocraftApp) -> Vec<photocraft_geom::Rect> {
        app.session.active().unwrap().doc.artboards().iter().map(|b| b.2.rect).collect()
    }

    /// Drawing on empty canvas makes a board; its right-edge handle resizes it in one undo step;
    /// dragging it from inside moves it.
    #[test]
    fn artboard_tool_draws_resizes_and_moves_boards() {
        let mut app = app();
        drag(&mut app, [20.0, 30.0], [120.0, 110.0]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 120, 110)]);
        let st = app.session.active().unwrap();
        assert_eq!(st.active_layer, Some(st.doc.artboards()[0].0));
        // The right edge's handle.
        drag(&mut app, [120.0, 70.0], [160.0, 75.0]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 160, 110)]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 120, 110)]);
        // From inside: the board moves.
        drag(&mut app, [60.0, 60.0], [70.0, 50.0]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(30, 20, 130, 100)]);
        // A click on empty canvas draws nothing.
        drag(&mut app, [300.0, 250.0], [300.0, 250.0]);
        assert_eq!(boards(&app).len(), 1);
    }

    /// The handles' resize arrows turn with a rotated view: on a view turned 90°, the right edge's
    /// handle resizes up and down on screen.
    #[test]
    fn resize_cursors_turn_with_the_view() {
        use CursorIcon::{ResizeHorizontal as H, ResizeNeSw as NeSw, ResizeNwSe as NwSe, ResizeVertical as V};
        let mut app = app();
        drag(&mut app, [20.0, 30.0], [120.0, 110.0]);
        // The right edge, the top edge, the top-left and the top-right corner.
        let handles = [[120.0, 70.0], [70.0, 30.0], [20.0, 30.0], [120.0, 30.0]];
        for (rotation, want) in [(0.0, [H, V, NwSe, NeSw]), (90.0, [V, H, NeSw, NwSe])] {
            let view_index = app.session.active_index().unwrap();
            app.ui.views[view_index].rotation = rotation;
            let xf = ViewXform::active(&app).unwrap();
            let got = handles.map(|p| cursor(&app, &xf, p));
            assert_eq!(got, want.map(Some), "view turned {rotation}°");
        }
    }

    /// #3033: a click on the selected board's right "+" adds a board of its size a gap to the right;
    /// ⇧ on a corner keeps the board's proportions.
    #[test]
    fn plus_buttons_add_boards_and_shift_keeps_proportions() {
        let mut app = app();
        drag(&mut app, [20.0, 30.0], [120.0, 80.0]);
        let (_, buttons) = plus_buttons(&app).unwrap();
        let right = buttons.iter().find(|b| b.0 == "right").unwrap().1;
        assert!(right[0] > 120.0 && (right[1] - 55.0).abs() < 1e-9, "{right:?}");
        drag(&mut app, right, right);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 120, 80), photocraft_geom::Rect::new(220, 30, 320, 80)]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app).len(), 1);
        // The 100×50 board's bottom-right corner, dragged 40 right and 5 down with ⇧: 140×70.
        let m = egui::Modifiers::SHIFT;
        tool_event(&mut app, ToolEvent::Down { x: 120.0, y: 80.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 160.0, y: 85.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 160.0, y: 85.0 }, m);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 160, 100)]);
    }

    /// With View › Snap on, a board drawn near the canvas corners snaps to them, a dragged edge
    /// snaps to the canvas edge (only on its own axis), and a board moved near the left edge
    /// snaps its left edge to it.
    #[test]
    fn boards_snap_to_the_canvas_when_drawn_resized_and_moved() {
        let mut app = app();
        app.ui.extras.snap = true;
        let tol = crate::snap_ui::tolerance(&app);
        assert!(tol > 2.5, "snap tolerance {tol} document px");
        drag(&mut app, [2.0, 2.0], [398.0, 150.0]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(0, 0, 400, 150)]);
        app.run("edit.undo", json!({})).unwrap();
        drag(&mut app, [20.0, 30.0], [120.0, 110.0]);
        // The right edge's handle, grabbed 2 px inside the edge, dragged near the canvas's right edge.
        drag(&mut app, [118.0, 70.0], [396.0, 73.0]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 400, 110)]);
        // From inside: moved 18 px left, 2 px from the canvas edge: it snaps to x = 0.
        drag(&mut app, [200.0, 70.0], [182.0, 70.0]);
        assert_eq!(boards(&app)[0].x0, 0);
        // Snap off: the same gestures land where the pointer is.
        app.ui.extras.snap = false;
        drag(&mut app, [200.0, 70.0], [203.0, 70.0]);
        assert_eq!(boards(&app)[0].x0, 3);
    }

    /// A board snapped to a guide on a half pixel lands on a whole one, and the snap line shows
    /// where it landed: 22.5 rounds up when drawing, and a 17.5 px move left rounds to 18.
    #[test]
    fn snap_lines_follow_the_board_onto_whole_pixels() {
        let mut app = app();
        app.ui.extras.snap = true;
        app.run("view.newGuide", json!({"orientation": "vertical", "position": 22.5})).unwrap();
        let m = egui::Modifiers::NONE;
        let line_x = |app: &PhotocraftApp| app.prefs_rt.snap_lines.iter().find(|l| l.vertical).map(|l| l.pos);
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 21.0, y: 110.0, pressure: 1.0 }, m);
        assert_eq!(line_x(&app), Some(23.0));
        tool_event(&mut app, ToolEvent::Up { x: 21.0, y: 110.0 }, m);
        assert_eq!(boards(&app)[0].x0, 23);
        app.run("edit.undo", json!({})).unwrap();
        drag(&mut app, [40.0, 30.0], [140.0, 110.0]);
        tool_event(&mut app, ToolEvent::Down { x: 90.0, y: 70.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 73.0, y: 70.0, pressure: 1.0 }, m);
        assert_eq!(line_x(&app), Some(22.0));
        tool_event(&mut app, ToolEvent::Up { x: 73.0, y: 70.0 }, m);
        assert_eq!(boards(&app)[0].x0, 22);
    }

    /// A board drawn around layers takes them in (the engine's `adopt`), in one undo step.
    #[test]
    fn a_drawn_board_takes_in_the_layers_inside_it() {
        let mut app = app();
        app.run("layer.new.layer", json!({"name": "Inside"})).unwrap();
        app.run("paint.pencil", json!({"points": [[50, 50], [80, 60]], "size": 6, "color": "#ff0000"})).unwrap();
        app.run("layer.new.layer", json!({"name": "Outside"})).unwrap();
        app.run("paint.pencil", json!({"points": [[300, 250], [380, 280]], "size": 6, "color": "#0000ff"})).unwrap();
        drag(&mut app, [20.0, 20.0], [150.0, 120.0]);
        let st = app.session.active().unwrap();
        let board = st.doc.artboards()[0].0;
        let names: Vec<&str> = st.doc.layer(board).unwrap().children().unwrap().iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Inside"]);
        assert!(st.doc.layers.iter().any(|l| l.name == "Outside"));
    }

    /// The Move tool drags a board by its outline (or the name strip above it) in one undo step;
    /// a press inside is left to the Move tool.
    #[test]
    fn the_move_tool_drags_a_board_by_its_edge() {
        let mut app = app();
        drag(&mut app, [20.0, 30.0], [120.0, 110.0]);
        app.ui.tool = Tool::Move;
        let steps = app.session.active().unwrap().history.past_len();
        // On the left edge.
        drag(&mut app, [20.0, 70.0], [60.0, 90.0]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(60, 50, 160, 130)]);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
        // Just above the top edge: the name strip.
        let above = 50.0 - 0.5 * LABEL_PX / f64::from(app.point_zoom());
        drag(&mut app, [100.0, above], [90.0, above]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(50, 50, 150, 130)]);
        assert!(app.ui.artboard_tool.drag.is_none());
        // Inside, away from the edges, the press is the Move tool's own (it moves the selected
        // layer), not a board grab.
        assert!(!move_tool_pointer(&mut app, ToolEvent::Down { x: 100.0, y: 90.0, pressure: 1.0 }, egui::Modifiers::NONE));
        assert!(app.ui.artboard_tool.drag.is_none());
        // A board drag whose release never came (let go outside the window) doesn't take over
        // the next press elsewhere: the later drag isn't the board's.
        let none = egui::Modifiers::NONE;
        assert!(move_tool_pointer(&mut app, ToolEvent::Down { x: 50.0, y: 90.0, pressure: 1.0 }, none));
        assert!(!move_tool_pointer(&mut app, ToolEvent::Down { x: 100.0, y: 90.0, pressure: 1.0 }, none));
        assert!(app.ui.artboard_tool.drag.is_none());
        assert!(!move_tool_pointer(&mut app, ToolEvent::Move { x: 120.0, y: 100.0, pressure: 1.0 }, none));
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(50, 50, 150, 130)]);
    }

    /// Resizing a board snaps its edge to the layers inside it (to fit it to the artwork) and to
    /// other boards' edges.
    #[test]
    fn resizing_snaps_to_the_contents_and_other_boards() {
        let mut app = app();
        app.ui.extras.snap = true;
        app.run("layer.new.layer", json!({"name": "Art"})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        app.run("select.rect", json!({"x": 40, "y": 40, "width": 61, "height": 40})).unwrap();
        app.run("select.inverse", json!({})).unwrap();
        app.run("edit.clear", json!({})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        // A board around the art (it takes the layer in), then its right edge dragged 2 px past
        // the art's right edge (x = 101).
        drag(&mut app, [20.0, 20.0], [150.0, 100.0]);
        assert_eq!(app.session.active().unwrap().doc.artboards()[0].2.rect, photocraft_geom::Rect::new(20, 20, 150, 100));
        drag(&mut app, [150.0, 60.0], [103.0, 60.0]);
        assert_eq!(boards(&app)[0].x1, 101);
        // A second board's left edge snaps to the first one's right edge.
        drag(&mut app, [200.0, 150.0], [300.0, 250.0]);
        drag(&mut app, [200.0, 200.0], [103.0, 200.0]);
        assert_eq!(boards(&app)[1].x0, 101);
    }

    /// V picks the Move group, ⇧V cycles Move ↔ Artboard as in Photoshop.
    #[test]
    fn shift_v_cycles_move_and_artboard() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let press = |app: &mut PhotocraftApp, modifiers| {
            let key = egui::Key::V;
            let raw = egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
                    egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
                ],
                ..Default::default()
            };
            ctx.begin_pass(raw);
            crate::shortcuts::handle(app, &ctx);
            ctx.end_pass().textures_delta.clear();
        };
        app.ui.tool = Tool::Brush;
        press(&mut app, egui::Modifiers::NONE);
        assert_eq!(app.ui.tool, Tool::Move);
        press(&mut app, egui::Modifiers::SHIFT);
        assert_eq!(app.ui.tool, Tool::Artboard);
        press(&mut app, egui::Modifiers::SHIFT);
        assert_eq!(app.ui.tool, Tool::Move);
    }
}
