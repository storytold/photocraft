//! The Ruler, Color Sampler, Count and Note tools on the canvas tool host, and what they paint over the document.
//!
//! The Ruler and the Color Sampler are the framework's tools (`RulerPass`, `ColorSamplerPass`); Count and Note are app
//! tools over the document's markers and notes. A tool reports its work as events; [`drain`] turns them into engine
//! commands after the host ran, once each, so a second pass in a frame (which gets no new pointer input) edits nothing.

use craft::behavior::kurbo::Point;
use craft::behavior::{CancelReason, CountMark, Cursor, CursorBadge, CursorIcon, EditEvent, Input, Metrics, Mixed, PaintedCursor};
use craft::color_ui::ui::{ColorSamplerMarks, ColorSamplerTool, ColorSamplers, SamplerEvent};
use craft::draw::ui::{RulerLine, RulerTool};
use craft::ui::canvas::{CanvasTool, NoTool, ToolAction, ToolClaims, ToolContext, ToolHostResponse, ToolKey, ToolPath, ToolPointer, canvas_space};
use craft::ui::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _};
use craft::ui::fonts::FontRef;
use craft::ui::primitives::{DocRun, DocText, Draw, PaintShape, StrokeWidth};
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2, vec2};
use serde_json::json;

use super::{create_note, show_note_options, status_on_err};
use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;
use crate::canvas::tools::{FamilyTool, ToolScope};
use crate::state::Tool;

/// The family's state between passes, kept in `CanvasTools::analysis`.
pub(crate) struct AnalysisTools {
    ruler: RulerTool,
    sampler: ColorSamplerTool,
    /// The document's points as the framework's collection, which the sampler pass borrows. It is rebuilt from the
    /// document at every pass (`tool`); it is storage for that borrow, not state.
    samplers: ColorSamplers,
    count: CountTool,
    note: NoteTool,
}

impl Default for AnalysisTools {
    fn default() -> Self {
        Self {
            ruler: RulerTool::new(),
            sampler: ColorSamplerTool::new(),
            samplers: ColorSamplers::new(),
            count: CountTool::default(),
            note: NoteTool::default(),
        }
    }
}

/// The document's Color Sampler points as the framework's collection, with the document's size in pixels. The one
/// place the document's list becomes `ColorSamplers`.
pub(super) fn samplers_of(app: &PhotocraftApp) -> Option<(ColorSamplers, [u32; 2])> {
    let st = app.session.active()?;
    let size = [st.doc.size.width, st.doc.size.height];
    let mut samplers = ColorSamplers::new();
    for p in &st.color_samplers {
        let _ = samplers.add(*p, size);
    }
    Some((samplers, size))
}

fn ruler_line(app: &PhotocraftApp) -> Option<RulerLine> {
    let r = app.session.active()?.doc.measurement.ruler?;
    let line = RulerLine::new(Point::new(r.start[0], r.start[1]), Point::new(r.end[0], r.end[1]));
    Some(match r.protractor {
        Some(q) => line.with_protractor(Point::new(q[0], q[1])),
        None => line,
    })
}

/// The tool of `Tool::{Ruler, ColorSampler, Count, Note}` for one pass.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut AnalysisTools, _host: CanvasHost) -> FamilyTool<'a> {
    let tool = scope.with_app(|app| app.ui.tool);
    let AnalysisTools { ruler, sampler, samplers, count, note } = state;
    match tool {
        Tool::Ruler => {
            let line = scope.with_app(|app| ruler_line(app));
            Box::new(ruler.pass(line))
        }
        // PLAN: with no document the old tool said "No document open" in the status line on a press; the host runs no
        // tool pass without a document, so the message has no caller here.
        Tool::ColorSampler => match scope.with_app(|app| samplers_of(app)) {
            Some((built, size)) => {
                *samplers = built;
                let samplers: &'a ColorSamplers = samplers;
                Box::new(sampler.pass(samplers, size))
            }
            None => Box::new(NoTool),
        },
        Tool::Count => Box::new(CountPass { scope, state: count }),
        _ => {
            // `claims(&self)` has no app: the delete claim is read once here, for the pass.
            let deletable = scope.with_app(|app| super::note_alerts::deletable(app));
            Box::new(NotePass { scope, state: note, deletable })
        }
    }
}

/// Turns the events the tools reported into engine commands, in order.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut AnalysisTools, _response: &ToolHostResponse) {
    for event in state.ruler.take_events() {
        match event {
            // A drag's updates are not applied: the pass shows the running line from the tool's own proposal, and the
            // document takes the line once, at the commit.
            EditEvent::Commit { after, .. } => set_ruler(app, after),
            EditEvent::Set { after, .. } => set_ruler(app, after),
            // Escape: the line from before the press comes back.
            EditEvent::Cancel { before: Some(Mixed::Same(line)), .. } => set_ruler(app, line),
            _ => {}
        }
    }
    for event in state.sampler.take_events() {
        let result = match event {
            SamplerEvent::Add { at, .. } => {
                if app.session.active().is_some_and(|st| st.color_samplers.len() >= photocraft_engine::sampler_cmds::MAX_SAMPLERS) {
                    open_max_samplers_alert(app);
                    continue;
                }
                let r = app.run("view.colorSamplers.add", json!({"x": at[0], "y": at[1]}));
                if r.is_ok() {
                    reveal_info_panel(app);
                }
                r
            }
            SamplerEvent::Move { index, to, .. } => app.run("view.colorSamplers.move", json!({"index": index, "x": to[0], "y": to[1]})),
            SamplerEvent::Remove { index, .. } => app.run("view.colorSamplers.delete", json!({"index": index})),
            _ => continue,
        };
        status_on_err(app, result);
    }
    if let Some(_toast) = state.sampler.take_error() {
        open_max_samplers_alert(app);
    }
    for event in std::mem::take(&mut state.count.events) {
        match event {
            CountEvent::Add { at } => {
                // A marker that stays invisible looks like the click did nothing, so adding one brings View > Show >
                // Count back. It is view state, not a history step.
                let result = app.run("count.add", json!({"x": at[0], "y": at[1]}));
                app.ui.view.show.count |= result.is_ok();
                status_on_err(app, result);
            }
            CountEvent::Move { group, index, to } => {
                let result = app.run("count.move", json!({"group": group, "index": index, "to": to}));
                status_on_err(app, result);
            }
            CountEvent::Remove { group, index } => {
                let result = app.run("count.remove", json!({"group": group, "index": index}));
                status_on_err(app, result);
            }
        }
    }
    for event in std::mem::take(&mut state.note.events) {
        match event {
            NoteEvent::Create { at } => {
                let result = create_note(app, at[0], at[1]);
                status_on_err(app, result);
            }
            NoteEvent::Select { index } => {
                app.ui.analysis.note_selected = Some(index);
                let _ = crate::panel_docking::show_panel(app, "notes", Some(true));
                show_note_options(app, index);
            }
            NoteEvent::Move { index, to } => {
                let result = app.run("notes.set", json!({"index": index, "x": to[0], "y": to[1]}));
                status_on_err(app, result);
            }
            NoteEvent::AskDelete { index } => super::note_alerts::ask_delete_note(app, index),
        }
    }
}

/// The alert when the 11th Color Sampler is clicked (#11743).
#[cfg(test)]
pub const MAX_SAMPLERS_ALERT: &str = "Could not create a new sampler because the maximum number of samplers is 10.";

/// Open the alert dialog when the Color Sampler limit is reached (#11743).
fn open_max_samplers_alert(app: &mut PhotocraftApp) {
    app.ui.status.clear();
    app.ui.status_error = false;
    let message = crate::i18n::fmt(
        tl!("Could not create a new sampler because the maximum number of samplers is {max}."),
        &[("max", &photocraft_engine::sampler_cmds::MAX_SAMPLERS.to_string())],
    );
    let _ = crate::dialogs::alerts::alert(app, message);
}

/// Open the closed Info panel when a Color Sampler point is placed (#11745).
pub(crate) fn reveal_info_panel(app: &mut PhotocraftApp) {
    if !crate::panel_docking::is_checked(app, "info") {
        let _ = crate::panel_docking::show_panel(app, "info", Some(true));
    }
}

/// Stores the Ruler line in the document: `None` clears it.
fn set_ruler(app: &mut PhotocraftApp, line: Option<RulerLine>) {
    let params = match line {
        Some(l) => json!({"start": [l.start.x, l.start.y], "end": [l.end.x, l.end.y], "protractor": l.protractor.map(|p| [p.x, p.y])}),
        None => json!({"clear": true}),
    };
    let result = app.run("image.analysis.rulerTool", params);
    status_on_err(app, result);
}

/// The document point under screen point `pos`.
fn doc_at(cx: &ToolContext<'_>, pos: Pos2) -> Option<[f64; 2]> {
    let p = cx.space()?.view.to_doc(pos);
    (p.x.is_finite() && p.y.is_finite()).then_some([p.x, p.y])
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The centre of the document pixel that holds `at`, where the Count tool puts a marker (#11761).
fn pixel_centre(at: [f64; 2]) -> [f64; 2] {
    [at[0].floor() + 0.5, at[1].floor() + 0.5]
}

/// The ruler line, the sampler marks, the count markers and the note cards over the document, in the canvas body over
/// `rect`. The marks show with every tool; the line with its handles only with the Ruler. View › Extras off hides the
/// line (View › Show has no Ruler entry, #11987) and the sampler marks, whose points and Info readouts stay (#11741).
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: Rect) {
    let view = &app.ui.view;
    let (show_count, show_notes, extras) = (view.shows(view.show.count), view.shows(view.show.notes), view.extras);
    let line = ruler_line(app).filter(|_| extras);
    let samplers = samplers_of(app).map(|(s, _)| s).filter(|_| extras);
    let groups: Vec<CountGroupMarks> = if show_count {
        app.session.active().map_or_else(Vec::new, |d| {
            d.doc
                .measurement
                .count_groups
                .iter()
                .filter(|g| g.visible)
                .map(|g| CountGroupMarks {
                    color: color32(g.color.to_rgb()),
                    radius: 1.0 + g.marker_size as f32,
                    label_size: g.label_size as f32,
                    points: g.points.clone(),
                })
                .collect()
        })
    } else {
        Vec::new()
    };
    let cards: Vec<NoteCard> = if show_notes {
        app.session.active().map_or_else(Vec::new, |d| d.doc.notes.iter().map(|n| NoteCard { at: n.position, color: color32(n.color.to_rgb()) }).collect())
    } else {
        Vec::new()
    };
    let selected = app.ui.analysis.note_selected;
    let tools = &mut app.canvas_tools.analysis;
    if app.ui.tool == Tool::Ruler {
        tools.ruler.pass(line).show_line(ui, rect);
    } else if let Some(line) = line {
        // The line and its arm stay drawn under every tool (#11987); only the Ruler tool has handles on them.
        craft::draw::ui::show_ruler(ui, rect, line);
    }
    if let Some(samplers) = &samplers {
        let mut marks = ColorSamplerMarks::new(tl!("Color samplers"), samplers);
        if let Some(carried) = tools.sampler.carried() {
            marks = marks.carried(carried);
        }
        marks.show_at(ui, rect);
    }
    if show_count {
        CountMarks::new(tl!("Count markers"), groups).show_at(ui, rect);
    }
    if show_notes {
        let mut note_cards = NoteCards::new(tl!("Notes"), cards);
        if let Some(selected) = selected {
            note_cards = note_cards.selected(selected);
        }
        note_cards.show_at(ui, rect);
    }
}

fn color32(c: [f32; 3]) -> Color32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    Color32::from_rgb(byte(c[0]), byte(c[1]), byte(c[2]))
}

// ------------------------------------------------------------------ Count

/// What the Count tool asks of the document.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CountEvent {
    Add { at: [f64; 2] },
    Move { group: usize, index: usize, to: [f64; 2] },
    Remove { group: usize, index: usize },
}

/// The Count tool's state: the marker a drag holds and the events not yet drained.
#[derive(Debug, Default)]
pub(crate) struct CountTool {
    /// The marker being dragged: group, index and where the marker was at the press.
    moving: Option<(usize, usize, [f64; 2])>,
    events: Vec<CountEvent>,
}

/// The visible marker nearest `at` within `tol` document units: (group, index). The press and the cursor both ask, so
/// the cursor never says "move" where a press adds a marker (#11773).
pub(super) fn count_marker_at(app: &PhotocraftApp, at: [f64; 2], tol: f64) -> Option<(usize, usize)> {
    let d = &app.session.active()?.doc;
    let mut best: Option<(f64, usize, usize)> = None;
    for (gi, g) in d.measurement.count_groups.iter().enumerate().filter(|(_, g)| g.visible) {
        for (pi, q) in g.points.iter().enumerate() {
            let dd = dist(*q, at);
            if dd <= tol && best.is_none_or(|b| dd < b.0) {
                best = Some((dd, gi, pi));
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

/// The Count tool for one pass.
struct CountPass<'a> {
    scope: &'a ToolScope<'a>,
    state: &'a mut CountTool,
}

impl CountPass<'_> {
    /// The marker under screen point `pos`, within the pointer's reach.
    fn hit(scope: &ToolScope<'_>, cx: &ToolContext<'_>, pos: Pos2) -> Option<(usize, usize)> {
        let space = cx.space()?;
        let at = doc_at(cx, pos)?;
        scope.with_app(|app| count_marker_at(app, at, space.view.doc_len(cx.reach())))
    }
}

impl CanvasTool for CountPass<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(at) = doc_at(cx, pointer.pos) else { return ToolAction::Ignored };
        match Self::hit(self.scope, cx, pointer.pos) {
            Some((group, index)) if pointer.mods.alt => {
                self.state.events.push(CountEvent::Remove { group, index });
                ToolAction::Done
            }
            Some((group, index)) => {
                let marker =
                    self.scope.with_app(|app| app.session.active().and_then(|st| st.doc.measurement.count_groups.get(group)?.points.get(index).copied()));
                let Some(marker) = marker else { return ToolAction::Ignored };
                self.state.moving = Some((group, index, marker));
                ToolAction::Claimed
            }
            None => {
                if !pointer.mods.alt {
                    self.state.events.push(CountEvent::Add { at: pixel_centre(at) });
                }
                ToolAction::Done
            }
        }
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some((group, index, from)) = self.state.moving.take() else { return };
        // A plain click on a marker re-centres it on the clicked pixel too; a marker already there moves nothing.
        if let Some(to) = doc_at(cx, pointer.pos).map(pixel_centre).filter(|to| *to != from) {
            self.state.events.push(CountEvent::Move { group, index, to });
        }
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.state.moving = None;
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let hit = cx.pointer.as_ref().and_then(|p| Self::hit(self.scope, cx, p.pos));
        match (hit, cx.mods.alt) {
            (Some(_), true) => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Count(CountMark::Delete)).into(),
            (Some(_), false) => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Count(CountMark::Move)).into(),
            (None, false) => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Count(CountMark::Add)).into(),
            (None, true) => Cursor::System(CursorIcon::Default),
        }
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the count.add command")
    }
}

// ------------------------------------------------------------------ Note

/// What the Note tool asks of the document.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NoteEvent {
    Create { at: [f64; 2] },
    Select { index: usize },
    Move { index: usize, to: [f64; 2] },
    AskDelete { index: usize },
}

/// The Note tool's state: the note a drag holds and the events not yet drained.
#[derive(Debug, Default)]
pub(crate) struct NoteTool {
    /// The note being dragged and the grab offset from its position.
    moving: Option<(usize, [f64; 2])>,
    events: Vec<NoteEvent>,
}

/// The Note tool for one pass.
struct NotePass<'a> {
    scope: &'a ToolScope<'a>,
    state: &'a mut NoteTool,
    /// The note a Delete would ask about, read once for the pass.
    deletable: Option<usize>,
}

impl CanvasTool for NotePass<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let (Some(at), Some(space)) = (doc_at(cx, pointer.pos), cx.space()) else { return ToolAction::Ignored };
        // The icon is 16 by 20 screen points, with its top left at the note's position.
        let (w, h) = (space.view.doc_len(16.0), space.view.doc_len(20.0));
        let hit = self.scope.with_app(|app| {
            let notes = &app.session.active()?.doc.notes;
            let i = notes.iter().rposition(|n| at[0] >= n.position[0] && at[0] <= n.position[0] + w && at[1] >= n.position[1] && at[1] <= n.position[1] + h)?;
            Some((i, [at[0] - notes[i].position[0], at[1] - notes[i].position[1]]))
        });
        match hit {
            Some((index, offset)) => {
                self.state.events.push(NoteEvent::Select { index });
                self.state.moving = Some((index, offset));
                ToolAction::Claimed
            }
            None => {
                self.state.events.push(NoteEvent::Create { at });
                ToolAction::Done
            }
        }
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some((index, offset)) = self.state.moving.take() else { return };
        let Some(at) = doc_at(cx, pointer.pos) else { return };
        let to = [(at[0] - offset[0]).round(), (at[1] - offset[1]).round()];
        // A press and release on one spot moves nothing.
        let from = self.scope.with_app(|app| app.session.active().and_then(|d| d.doc.notes.get(index)).map(|n| n.position));
        if from.is_some_and(|from| dist(from, to) >= 1.0) {
            self.state.events.push(NoteEvent::Move { index, to });
        }
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.state.moving = None;
    }

    fn claims(&self) -> ToolClaims {
        if self.deletable.is_some() { ToolClaims::NONE.deletes() } else { ToolClaims::NONE }
    }

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        match (key, self.deletable) {
            (ToolKey::Delete { .. }, Some(index)) => {
                self.state.events.push(NoteEvent::AskDelete { index });
                true
            }
            _ => false,
        }
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the notes.add command")
    }
}

// ------------------------------------------------------------------ paint

/// The width a marker's number is laid out in, wide enough for any count at the largest Label Size.
const NUMBER_ROOM: f32 = 1000.0;

/// One visible count group's marks.
struct CountGroupMarks {
    color: Color32,
    radius: f32,
    /// The numbers' font size in screen points: the group's Label Size.
    label_size: f32,
    points: Vec<[f64; 2]>,
}

/// The style of the count markers and note cards: the number's ink and the halo under it.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct OverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    ink: StyleColor,
    halo: StyleColor,
    /// The number's shadow offset.
    shadow: f32,
}

fn overlay_style(cx: &StyleCx<'_>) -> OverlayStyle {
    let tokens = cx.tokens;
    OverlayStyle { metrics: Metrics::new(), ink: cx.color(tokens.color.overlay.stroke), halo: cx.color(tokens.color.overlay.halo), shadow: tokens.stroke.thin }
}

/// The count markers of the visible groups, numbered in each group, the numbers at the group's Label Size. Paint only.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
pub(crate) struct CountMarks {
    #[common]
    common: Common,
    groups: Vec<CountGroupMarks>,
}

impl CountMarks {
    fn new(label: impl Into<Label>, groups: Vec<CountGroupMarks>) -> Self {
        CountMarks { common: Common::new(), groups }.label(label)
    }
}

impl Component for CountMarks {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = OverlayStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: CountMarksStyleKey) -> OverlayStyle {
        overlay_style(cx)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let draw = ui.draw();
        let style = &cx.style;
        for group in &self.groups {
            for (i, p) in group.points.iter().enumerate() {
                let at = space.view.to_screen(Point::new(p[0], p[1]));
                draw.data_fill(&PaintShape::Circle(at, group.radius), DataColor::new(group.color));
                let number = (i + 1).to_string();
                let anchor = at + vec2(2.0 + group.radius, -1.0);
                for (offset, color) in [(Vec2::splat(style.shadow), style.halo), (Vec2::ZERO, style.ink)] {
                    let runs = [DocRun::new(number.as_str(), FontRef::proportional(), group.label_size, DataColor::new(color.egui()))];
                    let text = DocText::new(&runs);
                    let height = cx.layout_doc(&text).size().y;
                    let top_left = anchor + offset - vec2(0.0, height);
                    draw.doc(Rect::from_min_size(top_left, vec2(NUMBER_ROOM, height)), &text);
                }
            }
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// One note's icon.
struct NoteCard {
    at: [f64; 2],
    color: Color32,
}

/// The note icons, the selected one outlined heavier. Paint only.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
pub(crate) struct NoteCards {
    #[common]
    common: Common,
    cards: Vec<NoteCard>,
    /// The note shown in the Notes panel.
    #[prop]
    selected: Option<usize>,
}

impl NoteCards {
    fn new(label: impl Into<Label>, cards: Vec<NoteCard>) -> Self {
        NoteCards { common: Common::new(), cards, selected: None }.label(label)
    }
}

impl Component for NoteCards {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = OverlayStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: NoteCardsStyleKey) -> OverlayStyle {
        overlay_style(cx)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let draw = ui.draw();
        let style = &cx.style;
        let corner = CornerRadius::same(2);
        for (i, card) in self.cards.iter().enumerate() {
            let p = space.view.to_screen(Point::new(card.at[0], card.at[1]));
            let r = Rect::from_min_size(p, vec2(16.0, 20.0));
            draw.data_fill(&PaintShape::Rect(r, corner), DataColor::new(card.color));
            let selected = self.selected == Some(i);
            let (width, color) = if selected { (StrokeWidth::Medium, style.ink) } else { (StrokeWidth::Hairline, style.halo) };
            draw.stroke(&PaintShape::Rect(r, corner), width, color);
            // Folded corner and text lines.
            draw.stroke(&PaintShape::Path(vec![r.right_top() + vec2(-5.0, 0.0), r.right_top() + vec2(0.0, 5.0)]), StrokeWidth::Hairline, style.halo);
            for k in 0..3 {
                let y = r.top() + 7.0 + 4.0 * k as f32;
                draw.stroke(&PaintShape::Path(vec![Pos2::new(r.left() + 3.0, y), Pos2::new(r.right() - 3.0, y)]), StrokeWidth::Hairline, style.halo);
            }
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
