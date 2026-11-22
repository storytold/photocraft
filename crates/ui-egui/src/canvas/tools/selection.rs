//! The selection family: the Marquees, the Lasso, the Polygonal Lasso, the Magnetic Lasso and the Magic Wand.
//!
//! The framework tools draw their outlines and report [`SelectEvent`]s; this module wraps them for the host
//! ([`tool`]), turns the events into engine commands after the run ([`drain`]), and answers the questions other
//! parts of the app ask about a selection under way ([`SelectionTools::outline_open`], [`SelectionTools::drag_offset`],
//! [`shown_offset`], [`floating_outline`]).

use std::cell::RefCell;

use craft::behavior::geometry::{Constraint, SelectionOperation};
use craft::behavior::kurbo::{Point, Rect};
use craft::behavior::{CancelReason, Cursor, EditEvent, Mixed, PixelSize, PointerButton};
use craft::imaging::ui::select::{
    DragKind, LassoTool, MagneticLassoTool, MarqueeKind, MarqueeTool, PolygonLassoTool, PressRule, SelectEvent, SelectPhase, SelectionDrag, SelectionProbe,
    SelectionShape, drag_kind,
};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolHostResponse, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;
use serde_json::{Value, json};

use super::{FamilyTool, ToolScope};
use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;
use crate::magnetic_lasso_ui::{self, PcEdges};
use crate::state::Tool;

/// The state of every selection tool, kept across passes in `CanvasTools`.
pub(crate) struct SelectionTools {
    /// The Rectangular, Elliptical, Single Row and Single Column Marquees.
    pub(crate) marquee: MarqueeTool,
    /// The Lasso.
    pub(crate) lasso: LassoTool,
    /// The Polygonal Lasso.
    pub(crate) polygon: PolygonLassoTool,
    /// The Magnetic Lasso.
    pub(crate) magnetic: MagneticLassoTool<PcEdges>,
    /// The Magic Wand.
    pub(crate) wand: WandState,
    /// The refusal message of the running pixel drag has been shown (it shows once per drag, at the first move).
    lock_prompted: bool,
}

impl Default for SelectionTools {
    fn default() -> Self {
        Self {
            marquee: MarqueeTool::new(MarqueeKind::Rectangle),
            lasso: LassoTool::new(),
            polygon: PolygonLassoTool::new(),
            // PLAN: assumed `PcEdges: Default` (tools-w64).
            magnetic: MagneticLassoTool::new(PcEdges::default()),
            wand: WandState::default(),
            lock_prompted: false,
        }
    }
}

impl SelectionTools {
    /// A Lasso, Polygonal Lasso or Magnetic Lasso outline is open.
    pub(crate) fn outline_open(&self) -> bool {
        [self.lasso.phase(), self.polygon.phase(), self.magnetic.phase()].iter().any(|p| matches!(p, SelectPhase::Outline { .. }))
    }

    /// A Polygonal or Magnetic Lasso outline being drawn hides the selection it replaces.
    pub(crate) fn replaces_selection(&self) -> bool {
        self.polygon.replaces_selection() || self.magnetic.replaces_selection()
    }

    /// Ends the open outline of every lasso that is not `tool`, without a command: an outline lasts while its own tool
    /// is the selected one. The document is the draft's own check (`OutlineDraft::for_document`, each pass of its tool).
    pub(crate) fn end_outlines_but(&mut self, tool: Tool) {
        if tool != Tool::Lasso {
            self.lasso.reset();
        }
        if tool != Tool::PolygonLasso {
            self.polygon.reset();
        }
        if tool != Tool::MagneticLasso {
            self.magnetic.reset();
        }
    }

    /// The whole-pixel offset of the selection drag in progress, from the tool that runs it.
    pub(crate) fn drag_offset(&self) -> Option<(i32, i32)> {
        [self.marquee.phase(), self.lasso.phase(), self.polygon.phase(), self.magnetic.phase(), self.wand.drag.phase()].into_iter().find_map(drag_delta)
    }
}

/// The whole-pixel offset of a selection drag in `phase`, if one is under way.
pub(crate) fn drag_delta(phase: SelectPhase) -> Option<(i32, i32)> {
    match phase {
        SelectPhase::Dragging(drag) => Some((drag.delta[0], drag.delta[1])),
        _ => None,
    }
}

/// The drag of a press inside the selection for the app's own tools that select (the Magic Wand, and Object Selection
/// with Object Finder on): a marquee runs it, so one mechanism drags every selection, and its events go through
/// [`apply`] like the marquee's own. The tool decides which presses it gets.
pub(crate) struct InsideSelectionDrag {
    /// Runs the outline and pixel drags.
    marquee: MarqueeTool,
    /// `marquee` owns the gesture, from its press to its release or cancel.
    active: bool,
    /// The refusal message of the running pixel drag has been shown.
    lock_prompted: bool,
}

impl Default for InsideSelectionDrag {
    fn default() -> Self {
        Self { marquee: MarqueeTool::new(MarqueeKind::Rectangle), active: false, lock_prompted: false }
    }
}

impl InsideSelectionDrag {
    /// The options bar's operation, which decides whether a plain press inside the selection drags the outline.
    pub(crate) fn operation(&mut self, bar: SelectionOperation) {
        self.marquee.operation(bar);
    }

    /// The drag owns the gesture.
    pub(crate) fn active(&self) -> bool {
        self.active
    }

    /// Where the drag is, for [`SelectionTools::drag_offset`] and the retouch family's offset.
    pub(crate) fn phase(&self) -> SelectPhase {
        self.marquee.phase()
    }

    pub(crate) fn each_pass(&mut self, probe: &dyn SelectionProbe, cx: &mut ToolContext<'_>) {
        self.marquee.pass(probe).each_pass(cx);
    }

    /// Starts the drag; the caller has checked that the press is inside the selection and drags it.
    pub(crate) fn press(&mut self, probe: &dyn SelectionProbe, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let action = self.marquee.pass(probe).press(pointer, cx);
        self.active = action == ToolAction::Claimed;
        action
    }

    pub(crate) fn drag(&mut self, probe: &dyn SelectionProbe, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if self.active {
            self.marquee.pass(probe).drag(pointer, cx);
        }
    }

    pub(crate) fn release(&mut self, probe: &dyn SelectionProbe, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if std::mem::take(&mut self.active) {
            self.marquee.pass(probe).release(pointer, cx);
        }
    }

    pub(crate) fn cancel(&mut self, probe: &dyn SelectionProbe, reason: CancelReason, cx: &mut ToolContext<'_>) {
        if std::mem::take(&mut self.active) {
            self.marquee.pass(probe).cancel(reason, cx);
        }
    }

    /// The marquee's cursor over the selection: the move cursor for the outline, kept while the drag runs.
    pub(crate) fn cursor(&mut self, probe: &dyn SelectionProbe, cx: &ToolContext<'_>) -> Cursor {
        self.marquee.pass(probe).cursor(cx)
    }

    /// Runs what the drag reported during the run; `tool` is the tool whose drag it was.
    pub(crate) fn drain(&mut self, app: &mut PhotocraftApp, tool: Tool) {
        for event in self.marquee.take_events() {
            apply(app, &mut self.lock_prompted, tool, event);
        }
    }
}

/// The Magic Wand: a click tool of the app. A press inside the selection that drags it runs on `drag`; any other press
/// is a click, answered after the run.
#[derive(Default)]
pub(crate) struct WandState {
    /// The drag of a press inside the selection.
    drag: InsideSelectionDrag,
    /// The clicks of this pass, for [`drain`].
    clicks: Vec<WandClick>,
}

/// A Magic Wand click: where, and how it combines with the selection.
struct WandClick {
    at: Point,
    operation: SelectionOperation,
}

/// The active document as the selection tools ask about it. It owns a snapshot (the selection's tiles are shared, so
/// the copy is cheap) because the tool scope lends the app only inside a closure.
pub(crate) struct PcSelection {
    document: u64,
    size: PixelSize,
    has_selection: bool,
    selection: Option<photocraft_raster::Surface>,
    piece: Option<Piece>,
}

/// A floating piece: where it is shown and the outline of its pixels.
struct Piece {
    offset: (i32, i32),
    outline: Option<photocraft_raster::Surface>,
}

impl PcSelection {
    /// The snapshot of the active document.
    pub(crate) fn of(app: &PhotocraftApp) -> Self {
        let st = app.session.active();
        let (document, size) = st.map_or((0, PixelSize::new(0, 0)), |st| {
            let b = st.doc.bounds();
            (st.doc.id.0 ^ app.session.active_generation(), PixelSize::new(b.width(), b.height()))
        });
        let piece = st.and_then(photocraft_engine::float_cmds::floating).map(|f| Piece { offset: f.offset, outline: f.parts.outline().cloned() });
        Self { document, size, has_selection: crate::tool_feedback::has_selection(app), selection: st.and_then(|st| st.doc.selection.clone()), piece }
    }
}

/// Does selection surface `sel` cover document point `at`?
fn covers(sel: &photocraft_raster::Surface, at: Point) -> bool {
    let mut v = [0.0f32];
    sel.read_pixel(at.x.floor() as i32, at.y.floor() as i32, &mut v);
    v[0] > 0.0
}

impl SelectionProbe for PcSelection {
    fn document(&self) -> u64 {
        self.document
    }

    fn size(&self) -> PixelSize {
        self.size
    }

    fn has_selection(&self) -> bool {
        self.has_selection
    }

    fn inside_selection(&self, at: Point) -> bool {
        self.selection.as_ref().is_some_and(|sel| covers(sel, at))
    }

    fn bounds(&self) -> Option<Rect> {
        let b = self.selection.as_ref()?.content_bounds();
        (!b.is_empty()).then(|| Rect::new(f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)))
    }

    fn floating(&self) -> bool {
        self.piece.is_some()
    }

    fn on_floating(&self, at: Point) -> bool {
        let Some(piece) = &self.piece else { return self.inside_selection(at) };
        let at = Point::new(at.x - f64::from(piece.offset.0), at.y - f64::from(piece.offset.1));
        piece.outline.as_ref().map_or_else(|| self.inside_selection(at), |outline| covers(outline, at))
    }
}

/// Which selection tool runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Marquee,
    Lasso,
    Polygon,
    Magnetic,
    Wand,
}

fn which(tool: Tool) -> Which {
    match tool {
        Tool::Lasso => Which::Lasso,
        Tool::PolygonLasso => Which::Polygon,
        Tool::MagneticLasso => Which::Magnetic,
        Tool::MagicWand => Which::Wand,
        _ => Which::Marquee,
    }
}

fn marquee_kind(tool: Tool) -> MarqueeKind {
    match tool {
        Tool::EllipseMarquee => MarqueeKind::Ellipse,
        Tool::SingleRowMarquee => MarqueeKind::SingleRow,
        Tool::SingleColumnMarquee => MarqueeKind::SingleColumn,
        _ => MarqueeKind::Rectangle,
    }
}

fn marquee_tool(kind: MarqueeKind) -> Tool {
    match kind {
        MarqueeKind::Ellipse => Tool::EllipseMarquee,
        MarqueeKind::SingleRow => Tool::SingleRowMarquee,
        MarqueeKind::SingleColumn => Tool::SingleColumnMarquee,
        _ => Tool::RectMarquee,
    }
}

/// The options bar's Style, Width and Height as the marquee's constraint.
fn constraint(app: &PhotocraftApp) -> Constraint {
    let o = &app.ui.tool_options;
    let (w, h) = (f64::from(o.marquee_width), f64::from(o.marquee_height));
    match o.marquee_style.as_str() {
        "fixedRatio" => Constraint::Ratio(w, h),
        "fixedSize" => Constraint::Size(w, h),
        _ => Constraint::Free,
    }
}

/// The selection family's tool for this pass. Before each run the options bar's operation and style are set on the
/// tools.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut SelectionTools, _host: CanvasHost) -> FamilyTool<'a> {
    let (probe, tool, bar, constraint) = scope.with_app(|app| {
        let tool = app.ui.tool;
        if tool == Tool::MagneticLasso {
            // PLAN: assumed `magnetic_lasso_ui::prepare(app: &mut PhotocraftApp, tool: &mut MagneticLassoTool<PcEdges>)` (tools-w64).
            magnetic_lasso_ui::prepare(app, &mut state.magnetic);
        }
        (PcSelection::of(app), tool, crate::tool_feedback::selection_op(tool, app.ui.selection_mode, Mods::NONE, true), constraint(app))
    });
    state.marquee.kind(marquee_kind(tool)).operation(bar).constraint(constraint);
    state.lasso.operation(bar);
    state.polygon.operation(bar);
    state.magnetic.operation(bar);
    state.wand.drag.operation(bar);
    Box::new(SelectionPass { scope, state: RefCell::new(state), probe, which: which(tool), bar })
}

/// The pass's tool: the chosen framework tool built over this pass's probe in each call (the probe lives here, the
/// pass borrows it). A press without Cmd first drops a floating piece (#11368).
struct SelectionPass<'a> {
    scope: &'a ToolScope<'a>,
    state: RefCell<&'a mut SelectionTools>,
    probe: PcSelection,
    which: Which,
    bar: SelectionOperation,
}

impl SelectionPass<'_> {
    /// Runs `f` on the chosen tool as a pass over this pass's probe.
    fn with<R>(&self, f: impl FnOnce(&mut dyn CanvasTool) -> R) -> R {
        let mut guard = self.state.borrow_mut();
        let s: &mut SelectionTools = &mut guard;
        match self.which {
            Which::Marquee => f(&mut s.marquee.pass(&self.probe)),
            Which::Lasso => f(&mut s.lasso.pass(&self.probe)),
            Which::Polygon => f(&mut s.polygon.pass(&self.probe)),
            Which::Magnetic => f(&mut s.magnetic.pass(&self.probe)),
            Which::Wand => f(&mut WandPass { state: &mut s.wand, probe: &self.probe, bar: self.bar }),
        }
    }
}

impl CanvasTool for SelectionPass<'_> {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.with(|t| t.each_pass(cx));
    }

    /// A press that does not drag the piece's pixels drops a floating piece where it is first; the probe is then
    /// taken again, so the tool sees the document without the piece.
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        if pointer.button == PointerButton::Primary
            && self.probe.floating()
            && let Some(at) = cx.space().map(|space| space.view.to_doc(pointer.pos))
            && !matches!(drag_kind(&self.probe, at, pointer.mods, self.bar, PressRule::Drags), Some(DragKind::Pixels { .. }))
        {
            self.probe = self.scope.with_app(|app| {
                run(app, "select.drop", json!({}));
                PcSelection::of(app)
            });
        }
        self.with(|t| t.press(pointer, cx))
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.with(|t| t.drag(pointer, cx));
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.with(|t| t.release(pointer, cx));
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.with(|t| t.hover(pointer, cx));
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        self.with(|t| t.cancel(reason, cx));
    }

    fn claims(&self) -> ToolClaims {
        self.with(|t| t.claims())
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        self.with(|t| t.key(key, cx))
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        self.with(|t| t.paint(draw, cx));
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        self.with(|t| t.cursor(cx))
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        self.with(|t| t.takes(button, mods))
    }

    fn autoscrolls(&self) -> bool {
        self.with(|t| t.autoscrolls())
    }

    fn autoscrolls_on_hover(&self) -> bool {
        self.with(|t| t.autoscrolls_on_hover())
    }

    fn view_only(&self) -> bool {
        self.with(|t| t.view_only())
    }

    fn takes_every_move(&self) -> bool {
        self.with(|t| t.takes_every_move())
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        self.with(|t| t.access(cx))
    }

    fn command_form(&self) -> ToolPath {
        self.with(|t| t.command_form())
    }
}

/// The Magic Wand for one pass. A press inside the selection that drags it (an outline in New mode, the pixels with
/// Cmd) goes to the wand's marquee; a Cmd press elsewhere is the Move stand-in's; any other press is a click.
struct WandPass<'a> {
    state: &'a mut WandState,
    probe: &'a PcSelection,
    bar: SelectionOperation,
}

impl WandPass<'_> {
    /// What a press at `at` drags. A floating piece is dragged only with Cmd: a plain press there samples (the piece
    /// is dropped first).
    fn drag_kind(&self, pointer: &ToolPointer, at: Point) -> Option<DragKind> {
        if self.probe.floating() && !pointer.mods.cmd {
            return None;
        }
        drag_kind(self.probe, at, pointer.mods, self.bar, PressRule::Drags)
    }
}

impl CanvasTool for WandPass<'_> {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.state.drag.each_pass(self.probe, cx);
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(at) = cx.space().map(|space| space.view.to_doc(pointer.pos)) else { return ToolAction::Ignored };
        if self.drag_kind(pointer, at).is_some() {
            return self.state.drag.press(self.probe, pointer, cx);
        }
        if pointer.mods.cmd {
            return ToolAction::Ignored;
        }
        let operation = SelectionOperation::resolve(self.bar, pointer.mods, self.probe.has_selection());
        self.state.clicks.push(WandClick { at, operation });
        ToolAction::Done
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.state.drag.drag(self.probe, pointer, cx);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.state.drag.release(self.probe, pointer, cx);
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        self.state.drag.cancel(self.probe, reason, cx);
    }

    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {}

    /// The marquee's selection drag takes every move from the press, as the marquee's own does.
    fn takes_every_move(&self) -> bool {
        self.state.drag.active()
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the Magic Wand command takes the same point")
    }
}

/// Runs an engine command; an error goes to the status line.
fn run(app: &mut PhotocraftApp, command: &str, params: Value) -> bool {
    match app.run(command, params) {
        Ok(_) => true,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            false
        }
    }
}

fn deselect(app: &mut PhotocraftApp) {
    if app.session.is_enabled("select.deselect") {
        run(app, "select.deselect", json!({}));
    }
}

fn piece_offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    app.session.active().and_then(photocraft_engine::float_cmds::floating).map(|f| f.offset)
}

/// The engine command of a selection a tool made: the one `SelectEvent::Commit` mapping. `tool` names the History
/// step (Single Row, Polygonal Lasso, Patch Tool) and picks the options the bar has for it.
pub(crate) fn run_commit(app: &mut PhotocraftApp, tool: Tool, shape: SelectionShape, op: SelectionOperation) {
    let mode = crate::tool_feedback::mode_name(op);
    let (anti_alias, feather) = (app.ui.tool_options.anti_alias, app.ui.tool_options.feather);
    let r = |v: f64| v.round() as i64;
    match shape {
        SelectionShape::Rectangle(rect) | SelectionShape::Ellipse(rect) => {
            let ellipse = tool == Tool::EllipseMarquee;
            let mut params = json!({"x": r(rect.x0), "y": r(rect.y0), "width": r(rect.x1 - rect.x0), "height": r(rect.y1 - rect.y0), "mode": mode, "ellipse": ellipse, "antiAlias": anti_alias, "feather": feather});
            // `line` names the History step after the tool; a line has no anti-aliasing.
            match tool {
                Tool::SingleRowMarquee => (params["line"], params["antiAlias"]) = (json!("row"), json!(false)),
                Tool::SingleColumnMarquee => (params["line"], params["antiAlias"]) = (json!("column"), json!(false)),
                _ => {}
            }
            run(app, "select.rect", params);
        }
        SelectionShape::Polygon(points) => {
            let points: Vec<[f64; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
            // The Patch and Content-Aware Move tools have no Feather in their options bar.
            let feather = if matches!(tool, Tool::Patch | Tool::ContentAwareMove) { 0.0 } else { feather };
            let mut params = json!({"points": points, "mode": mode, "antiAlias": anti_alias, "feather": feather});
            if tool == Tool::PolygonLasso {
                params["polygonal"] = json!(true);
            }
            if tool == Tool::Patch {
                params["historyName"] = json!("Patch Tool");
            }
            run(app, "select.lasso", params);
        }
        // `SelectionShape` is `#[non_exhaustive]`: a shape the app has no engine command for commits nothing.
        _ => {}
    }
}

/// Turns what the selection tools reported during the run into engine commands, in order. Each tool's events are
/// taken once, so a second pass in the same frame finds nothing left to run.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut SelectionTools, _response: &ToolHostResponse) {
    let marquee = marquee_tool(state.marquee.marquee_kind());
    for event in state.marquee.take_events() {
        apply(app, &mut state.lock_prompted, marquee, event);
    }
    for event in state.lasso.take_events() {
        apply(app, &mut state.lock_prompted, Tool::Lasso, event);
    }
    for event in state.polygon.take_events() {
        apply(app, &mut state.lock_prompted, Tool::PolygonLasso, event);
    }
    for event in state.magnetic.take_events() {
        apply(app, &mut state.lock_prompted, Tool::MagneticLasso, event);
    }
    state.wand.drag.drain(app, Tool::MagicWand);
    for click in std::mem::take(&mut state.wand.clicks) {
        wand_click(app, &click);
    }
    // PLAN: assumed `magnetic_lasso_ui::store(app: &mut PhotocraftApp, tool: &MagneticLassoTool<PcEdges>)` (tools-w64).
    magnetic_lasso_ui::store(app, &state.magnetic);
}

fn apply(app: &mut PhotocraftApp, prompted: &mut bool, tool: Tool, event: SelectEvent) {
    match event {
        SelectEvent::Commit { shape, operation, .. } => match (tool, shape) {
            // PLAN: assumed `magnetic_lasso_ui::commit(app, points: &[Point], mode: &str)` (tools-w64).
            (Tool::MagneticLasso, SelectionShape::Polygon(points)) => magnetic_lasso_ui::commit(app, &points, crate::tool_feedback::mode_name(operation)),
            (_, shape) => run_commit(app, tool, shape, operation),
        },
        SelectEvent::Deselect => deselect(app),
        SelectEvent::DragSelection(edit) => drag_event(app, prompted, edit),
        _ => {}
    }
}

/// A Magic Wand click: a New click off the document deselects (Photoshop's "Deselect", #11782), any other samples.
fn wand_click(app: &mut PhotocraftApp, click: &WandClick) {
    let (x, y) = (click.at.x.floor(), click.at.y.floor());
    let on_canvas = app.session.active().is_some_and(|st| st.doc.bounds().contains(x as i32, y as i32));
    if click.operation == SelectionOperation::New && !on_canvas {
        deselect(app);
        return;
    }
    let o = app.ui.tool_options.clone();
    let mode = crate::tool_feedback::mode_name(click.operation);
    run(
        app,
        "select.magicWand",
        json!({"x": x, "y": y, "tolerance": o.tolerance, "contiguous": o.contiguous, "antiAlias": o.anti_alias, "sampleAllLayers": o.sample_all.magic_wand, "mode": mode}),
    );
}

fn drag_event(app: &mut PhotocraftApp, prompted: &mut bool, edit: EditEvent<SelectionDrag>) {
    match edit {
        EditEvent::Begin { before: Mixed::Same(drag), .. } => {
            *prompted = false;
            begin_drag(app, drag.kind);
        }
        EditEvent::Update { value, .. } => refuse_pixel_drag(app, prompted, value),
        EditEvent::Commit { after, .. } => {
            *prompted = false;
            finish_drag(app, after);
        }
        EditEvent::Cancel { .. } => {
            *prompted = false;
            // A cut that never moved goes back, so Undo isn't spent on it.
            if piece_offset(app) == Some((0, 0)) {
                run(app, "select.drop", json!({}));
            }
        }
        _ => {}
    }
}

/// The press of a pixel drag cuts the selected pixels into a floating piece (Cmd+Alt: a copy, leaving a piece
/// already there where it is). A layer that cannot float refuses at the drag's first move instead (`refuse_pixel_drag`).
fn begin_drag(app: &mut PhotocraftApp, kind: DragKind) {
    let DragKind::Pixels { copy, .. } = kind else { return };
    let floating = piece_offset(app).is_some();
    if floating && !copy {
        return;
    }
    if floating && !run(app, "select.drop", json!({})) {
        return;
    }
    // PLAN: assumed `move_lock::pixels_locked(app: &PhotocraftApp) -> bool` (tools-w65).
    if crate::move_lock::pixels_locked(app) {
        return;
    }
    run(app, "select.float", json!({"dx": 0, "dy": 0, "copy": copy}));
}

/// The lock message of a pixel drag that could not cut: shown at the first move, never for a click.
fn refuse_pixel_drag(app: &mut PhotocraftApp, prompted: &mut bool, drag: SelectionDrag) {
    let pixels = matches!(drag.kind, DragKind::Pixels { .. });
    if pixels && drag.delta != [0, 0] && !*prompted && piece_offset(app).is_none() && crate::move_lock::pixels_locked(app) {
        crate::move_lock::prompt(app);
        *prompted = true;
    }
}

/// The release of a selection drag: the outline moves (`Move Selection`) or the piece does. A click deselects; a
/// click on a piece that never moved puts it back.
fn finish_drag(app: &mut PhotocraftApp, drag: SelectionDrag) {
    let pixels = matches!(drag.kind, DragKind::Pixels { .. });
    let [dx, dy] = drag.delta;
    let offset = piece_offset(app);
    match (pixels, dx == 0 && dy == 0) {
        (true, true) if offset == Some((0, 0)) => {
            run(app, "select.drop", json!({}));
        }
        (true, true) => {}
        (false, true) => deselect(app),
        // The cut was refused: nothing floats and nothing moves.
        (true, false) if offset.is_none() => {}
        (true, false) => {
            run(app, "select.float", json!({"dx": dx, "dy": dy}));
        }
        (false, false) => {
            run(app, "select.transformSelection", json!({"dx": dx, "dy": dy, "stepName": "Move Selection"}));
        }
    }
}

/// The selection a floating piece on document `idx` shows its ants for, before its offset: the moved pixels' outline,
/// as in Photoshop (#7538); `None` outlines the document's selection. The surface's tiles are shared, so the clone is
/// cheap.
pub(crate) fn floating_outline(app: &PhotocraftApp, idx: usize) -> Option<photocraft_raster::Surface> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    photocraft_engine::float_cmds::floating(app.session.active()?)?.parts.outline().cloned()
}

/// Where the selection is drawn relative to where it is: a floating piece's offset plus a drag in progress, else the
/// selection drag, else the offset of Move-tool layers that carry the selection along (#7538).
pub(crate) fn shown_offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    crate::move_ui::floating_offset(app)
        .or_else(|| app.canvas_tools.selection.drag_offset())
        .or_else(|| app.canvas_tools.retouch.drag_offset())
        .or_else(|| crate::move_ui::carries_selection(app).then(|| app.canvas_tools.mover.offset()).flatten())
}

#[cfg(test)]
mod tests {
    use egui::Modifiers;

    use super::*;
    use crate::canvas::{ToolEvent, tool_event};

    /// One release of a selection tool and the engine command it runs.
    struct Row {
        name: &'static str,
        tool: Tool,
        /// The options bar's operation: 0 New, 1 Add, 2 Subtract, 3 Intersect.
        bar: u8,
        mods: Modifiers,
        /// A rectangle selected first: x, y, width, height.
        existing: Option<[i32; 4]>,
        path: &'static [[f64; 2]],
        /// Each point is a click (the Polygonal Lasso) instead of a press, moves and a release.
        clicks: bool,
        command: &'static str,
        /// Parameters the command must carry.
        expect: fn() -> Value,
        /// Parameters it must not carry.
        absent: &'static [&'static str],
        step: Option<&'static str>,
    }

    const SQUARE: [i32; 4] = [10, 10, 20, 20];
    const FAR: [i32; 4] = [100, 100, 10, 10];

    fn rows() -> Vec<Row> {
        let row = |name, tool, command, path: &'static [[f64; 2]], expect: fn() -> Value| Row {
            name,
            tool,
            bar: 0,
            mods: Modifiers::NONE,
            existing: None,
            path,
            clicks: false,
            command,
            expect,
            absent: &[],
            step: None,
        };
        vec![
            row(
                "rectangle, New",
                Tool::RectMarquee,
                "select.rect",
                &[[20.0, 20.0], [60.0, 50.0]],
                || json!({"x": 20, "y": 20, "width": 40, "height": 30, "mode": "replace", "ellipse": false, "antiAlias": true, "feather": 3}),
            ),
            Row {
                mods: Modifiers::SHIFT,
                existing: Some(FAR),
                ..row(
                    "ellipse, Shift at the press adds",
                    Tool::EllipseMarquee,
                    "select.rect",
                    &[[20.0, 20.0], [60.0, 50.0]],
                    || json!({"mode": "add", "ellipse": true}),
                )
            },
            Row {
                bar: 2,
                existing: Some(FAR),
                ..row("the bar's Subtract", Tool::RectMarquee, "select.rect", &[[20.0, 20.0], [60.0, 50.0]], || json!({"mode": "subtract"}))
            },
            Row {
                step: Some("Single Row Marquee"),
                ..row(
                    "single row",
                    Tool::SingleRowMarquee,
                    "select.rect",
                    &[[120.3, 37.6]],
                    || json!({"x": 0, "y": 37, "width": 400, "height": 1, "mode": "replace", "line": "row", "antiAlias": false}),
                )
            },
            Row {
                mods: Modifiers::SHIFT,
                existing: Some(FAR),
                ..row(
                    "single column, Shift adds",
                    Tool::SingleColumnMarquee,
                    "select.rect",
                    &[[42.9, 80.0]],
                    || json!({"x": 42, "y": 0, "width": 1, "height": 300, "mode": "add", "line": "column", "antiAlias": false}),
                )
            },
            Row {
                absent: &["polygonal"],
                step: Some("Lasso"),
                ..row(
                    "freehand lasso, feather from the bar",
                    Tool::Lasso,
                    "select.lasso",
                    &[[40.0, 30.0], [160.0, 30.0], [160.0, 120.0], [40.0, 120.0]],
                    || json!({"mode": "replace", "antiAlias": true, "feather": 3}),
                )
            },
            Row {
                clicks: true,
                step: Some("Polygonal Lasso"),
                ..row(
                    "polygon closed on its start",
                    Tool::PolygonLasso,
                    "select.lasso",
                    &[[100.0, 100.0], [300.0, 100.0], [300.0, 200.0], [100.0, 100.0]],
                    || json!({"mode": "replace", "polygonal": true, "feather": 3}),
                )
            },
            Row {
                clicks: true,
                mods: Modifiers::SHIFT,
                existing: Some([350, 250, 20, 20]),
                ..row(
                    "polygon latches the mode at its first click",
                    Tool::PolygonLasso,
                    "select.lasso",
                    &[[100.0, 100.0], [300.0, 100.0], [300.0, 200.0], [100.0, 200.0], [100.0, 100.0]],
                    || json!({"mode": "add", "polygonal": true}),
                )
            },
            Row {
                existing: Some(SQUARE),
                step: Some("Deselect"),
                ..row("a click outside deselects", Tool::RectMarquee, "select.deselect", &[[200.0, 200.0]], || json!({}))
            },
            Row {
                existing: Some(SQUARE),
                step: Some("Deselect"),
                ..row("a click inside deselects (a drag of zero)", Tool::RectMarquee, "select.deselect", &[[20.0, 20.0]], || json!({}))
            },
            Row {
                existing: Some(SQUARE),
                step: Some("Move Selection"),
                ..row(
                    "a drag inside moves the outline",
                    Tool::RectMarquee,
                    "select.transformSelection",
                    &[[20.0, 20.0], [30.0, 25.0]],
                    || json!({"dx": 10, "dy": 5, "stepName": "Move Selection"}),
                )
            },
            Row {
                mods: Modifiers::COMMAND,
                existing: Some(SQUARE),
                ..row("Cmd drag inside floats the pixels", Tool::RectMarquee, "select.float", &[[20.0, 20.0], [35.0, 20.0]], || json!({"dx": 15, "dy": 0}))
            },
            Row {
                mods: Modifiers::COMMAND,
                existing: Some(SQUARE),
                ..row("a Cmd click inside drops the unmoved piece", Tool::RectMarquee, "select.drop", &[[20.0, 20.0]], || json!({}))
            },
            Row {
                step: Some("Magic Wand"),
                ..row("wand click", Tool::MagicWand, "select.magicWand", &[[50.0, 50.0]], || json!({"x": 50, "y": 50, "mode": "replace", "antiAlias": true}))
            },
            Row {
                existing: Some(SQUARE),
                ..row("wand click off the document in New deselects", Tool::MagicWand, "select.deselect", &[[-30.0, -30.0]], || json!({}))
            },
        ]
    }

    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.ui.tool_options.anti_alias = true;
        app.ui.tool_options.feather = 3.0;
        app
    }

    fn same(actual: &Value, expected: &Value) -> bool {
        match (actual.as_f64(), expected.as_f64()) {
            (Some(a), Some(e)) => (a - e).abs() < 1e-9,
            _ => actual == expected,
        }
    }

    fn drive(app: &mut PhotocraftApp, row: &Row) {
        let m = row.mods;
        let down = |app: &mut PhotocraftApp, p: [f64; 2]| tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, m);
        let up = |app: &mut PhotocraftApp, p: [f64; 2]| tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, m);
        if row.clicks {
            // Frames between the clicks pass egui's double-click delay, so no click pair is a double click.
            let delay = app.ctx().options(|o| o.input_options.max_double_click_delay);
            let frames = (delay * 60.0).ceil() as usize + 1;
            for &p in row.path {
                down(app, p);
                up(app, p);
                for _ in 0..frames {
                    crate::canvas::testing::frame(app);
                }
            }
            return;
        }
        down(app, row.path[0]);
        for &p in &row.path[1..] {
            tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, m);
        }
        up(app, *row.path.last().unwrap());
    }

    /// Every release of a selection tool runs the engine command the options bar and the modifiers at the press name:
    /// shape, mode, line, polygonal, feather, and the History step.
    #[test]
    fn a_release_runs_its_selection_command() {
        for row in rows() {
            let mut app = app();
            app.ui.tool = row.tool;
            app.ui.selection_mode = row.bar;
            if let Some([x, y, width, height]) = row.existing {
                app.run("select.rect", json!({"x": x, "y": y, "width": width, "height": height})).unwrap();
            }
            drive(&mut app, &row);
            assert!(!app.ui.status_error, "{}: {}", row.name, app.ui.status);
            let (command, params) = app.session.journal.last().expect("a command ran").clone();
            assert_eq!(command, row.command, "{}", row.name);
            for (key, want) in (row.expect)().as_object().unwrap() {
                assert!(same(&params[key.as_str()], want), "{}: {key} is {} not {want}", row.name, params[key.as_str()]);
            }
            for key in row.absent {
                assert!(params.get(*key).is_none(), "{}: {key} is set", row.name);
            }
            if let Some(step) = row.step {
                let label = app.session.active().unwrap().history.undo_label().map(str::to_string);
                assert_eq!(label.as_deref(), Some(step), "{}", row.name);
            }
        }
    }

    /// A New wand click off the document deselects; in Add mode it does not.
    #[test]
    fn a_wand_click_off_the_document_keeps_the_selection_in_add_mode() {
        let mut app = app();
        app.ui.tool = Tool::MagicWand;
        app.ui.selection_mode = 1;
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        tool_event(&mut app, ToolEvent::Down { x: -30.0, y: -30.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x: -30.0, y: -30.0 }, Modifiers::NONE);
        assert!(app.session.active().unwrap().doc.selection.is_some());
    }
}
