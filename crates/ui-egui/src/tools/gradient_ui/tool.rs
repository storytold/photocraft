//! The Gradient tool's canvas gesture: a drag draws a gradient. In live mode it makes a Gradient Fill layer
//! (`gradient.fill.create`) or redraws the active one (`gradient.fill.set`); in classic mode it paints the pixels
//! (`paint.gradient`). The handles of an existing gradient are the canvas body's (`live_overlay`), which takes a press
//! on them before it reaches this tool.

use craft::behavior::kurbo::Point;
use craft::behavior::{CancelReason, Cursor, CursorIcon, GesturePhase, PointerButton};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolContext, ToolHostResponse, ToolPath, ToolPointer};
use craft::ui::primitives::{DataWidth, Draw, PaintShape};
use craft::ui::style::DataColor;
use egui::{Color32, Pos2};
use photocraft_doc::LayerId;
use serde_json::json;

use super::{active_gradient, cmds, create_params, dist, live_mode};
use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;
use crate::canvas::tools::{FamilyTool, ToolScope};

/// The shortest drag, in screen points, that draws a gradient: a click changes nothing.
const MIN_DRAG: f32 = 2.0;

/// The drag in progress, kept in `CanvasTools::gradient`. Points are document pixels.
#[derive(Debug, Default)]
pub(crate) struct GradientDraw {
    /// Where the press landed; `None` while no drag runs.
    pub(crate) from: Option<[f32; 2]>,
    /// Where the pointer is now, ⇧-snapped to 45° from `from`.
    pub(crate) to: [f32; 2],
    /// The active gradient layer the drag redraws (live mode), else it makes a new one.
    pub(crate) redraw: Option<LayerId>,
    /// Whether the last sample snapped to 45°.
    pub(crate) shift: bool,
}

impl GradientDraw {
    fn clear(&mut self) {
        *self = GradientDraw::default();
    }
}

/// The Gradient tool for one pass.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut GradientDraw, _host: CanvasHost) -> FamilyTool<'a> {
    Box::new(GradientTool { scope, state })
}

/// Ends the drag when the host reports its end. The tool clears it at release and cancel itself; this covers a gesture
/// the host ended without a call, so none is left behind.
pub(crate) fn drain(_app: &mut PhotocraftApp, state: &mut GradientDraw, response: &ToolHostResponse) {
    if matches!(response.gesture, Some(GesturePhase::Commit | GesturePhase::Cancel)) {
        state.clear();
    }
}

struct GradientTool<'a> {
    scope: &'a ToolScope<'a>,
    state: &'a mut GradientDraw,
}

/// The document point under screen position `pos`.
fn doc_at(cx: &ToolContext<'_>, pos: Pos2) -> Option<[f32; 2]> {
    let p = cx.space()?.view.to_doc(pos);
    Some([p.x as f32, p.y as f32])
}

/// `to` moved onto the nearest 45° ray from `from` while ⇧ is held (the classic drag's snap).
fn snapped(from: [f32; 2], to: [f32; 2], shift: bool) -> [f32; 2] {
    if !shift {
        return to;
    }
    let c = crate::stroke_constraint::snap45([f64::from(from[0]), f64::from(from[1])], [f64::from(to[0]), f64::from(to[1])]);
    [c[0] as f32, c[1] as f32]
}

/// Runs the drag `from`→`to` as the active mode does: the live Gradient Fill layer, or the classic paint.
fn commit(app: &mut PhotocraftApp, from: [f32; 2], to: [f32; 2], redraw: Option<LayerId>) {
    if live_mode(app) {
        let result = match redraw {
            Some(id) => app.run(cmds::SET, json!({"layer": id.0, "from": from, "to": to})),
            None => {
                let p = create_params(app, from, to);
                app.run(cmds::CREATE, p)
            }
        };
        report(app, result);
        return;
    }
    let o = app.ui.tool_options.clone();
    let params = json!({
        "from": from, "to": to, "style": o.gradient_style, "reverse": o.gradient_reverse, "dither": o.gradient_dither,
        "opacity": o.fill_opacity, "mode": o.gradient_blend_mode.label(), "target": crate::canvas::paint_target(app),
    });
    let result = app.run("paint.gradient", params);
    report(app, result);
}

/// Says in the status line why a drag changed nothing (a locked layer, no pixels to paint).
fn report<T, E: std::fmt::Display>(app: &mut PhotocraftApp, result: Result<T, E>) {
    if let Err(error) = result {
        app.ui.status = error.to_string();
        app.ui.status_error = true;
    }
}

impl CanvasTool for GradientTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(at) = doc_at(cx, pointer.pos) else { return ToolAction::Ignored };
        self.state.redraw = self.scope.with_app(|app| if live_mode(app) { active_gradient(app).map(|(layer, ..)| layer.id) } else { None });
        self.state.from = Some(at);
        self.state.to = at;
        self.state.shift = pointer.mods.shift;
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (Some(from), Some(at)) = (self.state.from, doc_at(cx, pointer.pos)) else { return };
        self.state.shift = pointer.mods.shift;
        self.state.to = snapped(from, at, pointer.mods.shift);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.drag(pointer, cx);
        let space = cx.space();
        let (from, to, redraw) = (self.state.from, self.state.to, self.state.redraw);
        self.state.clear();
        let (Some(from), Some(space)) = (from, space) else { return };
        let screen = |p: [f32; 2]| space.view.to_screen(Point::new(f64::from(p[0]), f64::from(p[1])));
        if screen(from).distance(screen(to)) < MIN_DRAG {
            return;
        }
        self.scope.with_app(|app| commit(app, from, to, redraw));
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.state.clear();
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(from), Some(space)) = (self.state.from, cx.space()) else { return };
        let screen = |p: [f32; 2]| space.view.to_screen(Point::new(f64::from(p[0]), f64::from(p[1])));
        let (a, b) = (screen(from), screen(self.state.to));
        if a == b {
            return;
        }
        draw.stroke(&PaintShape::Path(vec![a, b]), DataWidth::points(3.0), DataColor::new(Color32::from_black_alpha(140)));
        draw.stroke(&PaintShape::Path(vec![a, b]), DataWidth::points(1.0), DataColor::new(Color32::WHITE));
        for end in [a, b] {
            draw.data_fill(&PaintShape::Circle(end, 3.0), DataColor::new(Color32::WHITE));
        }
    }

    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        Cursor::System(CursorIcon::Crosshair)
    }

    /// ⌘ is the Move tool's stand-in and ⌥ the Eyedropper's: with either held the press belongs to the stand-in.
    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        button == PointerButton::Primary && !mods.cmd && !crate::canvas::alt_samples(crate::state::Tool::Gradient, mods)
    }

    fn access(&self, _cx: &ToolContext<'_>) -> Option<ToolAccess> {
        let from = self.state.from?;
        let access = ToolAccess::new(tl!("Gradient").to_string()).value(format!("{:.0} px", dist(from, self.state.to)));
        Some(if self.state.shift { access.description(tl!("Snapped to 45°").to_string()) } else { access })
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the New Gradient Fill Layer command")
    }
}
