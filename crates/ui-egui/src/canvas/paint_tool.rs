//! The paint tool: every painting tool as one canvas tool for the tool host (`CanvasTool`).
//!
//! It serves the tools for which `tools::family` answers `Family::Paint`: the brush-like tools plus
//! Remove and Spot Healing. The host gives it each pointer move of a pass (`takes_every_move`); it
//! pushes them into the running [`PaintStroke`], and [`drain`] feeds the live stroke once per pass,
//! after the host has run, so the stroke shows the pass's moves in the same pass.
//!
//! The tool holds the [`ToolScope`] and reaches the app one call at a time (`with_app`), never two
//! at once. `PaintResize` lives in `app.brush_resize`: each call that needs the app too takes it out
//! with `std::mem::take` first.

use std::time::Duration;

use craft::behavior::{CancelReason, Cursor, CursorIcon, PaintedCursor, PointerButton};
use craft::keymap::{Chord, Key, Mods};
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolHostResponse, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::Draw;

use super::cursor_policy::{alt_loads_mixer, alt_samples, click_tool_cursor, paint_cursor_blocked, sample_target};
use super::document_display::CanvasHost;

use super::PhotocraftApp;
use super::paint_mouse::{SecondaryUse, clears, secondary_takes, sync_tool_brush};
use super::strokes::{
    HOLD_REPEAT_MS, begin_live_stroke, feed_live_stroke, live_stroke, live_stroke_busy, paint_target, snap_aliased_tip, stroke_params, stroke_wants_time,
    strokes_live, tap_at_peak_pressure,
};
use super::tools::{FamilyTool, ToolScope};
use crate::state::Tool;
use serde_json::json;

/// Plain Enter and Escape: what applies or drops the Remove tool's pending strokes.
const PENDING_KEYS: &[Chord] = &[Chord::plain(Key::Enter), Chord::plain(Key::Escape)];

/// A tool stroke being drawn: the fields of the gesture a painting press starts.
#[derive(Clone, Debug)]
pub(crate) struct PaintStroke {
    /// The tool the stroke paints with (after the Alt flips and Erase to History).
    pub(crate) tool: Tool,
    /// A History Brush or Erase to History stroke's options as at the press: a tool key during the
    /// drag changes the tool and its brush, not the stroke (`retouch_ui::history_brush_params`).
    pub(crate) history_params: Option<serde_json::Value>,
    /// Where the stroke began (the ⇧-click line's first point when it has one).
    pub(crate) start: [f64; 2],
    /// The stroke's points: x, y and pressure, in document pixels.
    pub(crate) points: Vec<[f64; 3]>,
    /// The modifiers held at the press (the latched ones included).
    pub(crate) modifiers: Mods,
    /// The modifiers held now (updated on every pointer of the gesture).
    pub(crate) live: Mods,
    /// Which of the modifiers held at the press have been released since.
    pub(crate) released: Mods,
    /// A Brush/Eraser stroke that erases: the Eraser, or a right-button Brush drag with
    /// Preferences › Tools › Right-click with painting tools set to Erase.
    pub(crate) erase: bool,
    /// A Brush or Pencil stroke in Photoshop's Clear mode (`paint_mouse::clears`, #3439): it
    /// erases, sent as `"mode": "clear"` so History names the tool rather than the Eraser.
    pub(crate) clear: bool,
    /// ⇧ constraint of the stroke (`stroke_constraint`).
    pub(crate) constrain: Option<crate::stroke_constraint::Axis>,
    /// A ⇧-click line from the previous stroke's end. Smoothing doesn't apply to it, so it ends
    /// at the click whatever Catch-up on Stroke End is set to, as in Photoshop (#11887).
    pub(crate) line: bool,
}

impl PaintStroke {
    /// A stroke of `tool` that began at `start` with `points`, pressed with `modifiers`.
    fn new(tool: Tool, start: [f64; 2], points: Vec<[f64; 3]>, modifiers: Mods, erase: bool) -> Self {
        Self { tool, history_params: None, start, points, modifiers, live: modifiers, released: Mods::NONE, erase, clear: false, constrain: None, line: false }
    }

    /// Records the modifiers of a pointer of the gesture.
    pub(crate) fn track(&mut self, mods: Mods) {
        self.live = mods;
        self.released.shift |= !mods.shift;
        self.released.alt |= !mods.alt;
    }
}

/// The paint family's state, in `CanvasTools::paint`.
#[derive(Debug, Default)]
pub(crate) struct PaintState {
    /// The stroke being drawn, from the press to the release.
    pub(crate) stroke: Option<PaintStroke>,
}

/// The paint tool for one pass.
pub(crate) struct PaintTool<'a> {
    scope: &'a ToolScope<'a>,
    state: &'a mut PaintState,
}

/// The paint family's tool for this pass.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut PaintState, _host: CanvasHost) -> FamilyTool<'a> {
    Box::new(PaintTool { scope, state })
}

/// The stroke being drawn, for readers outside the host's run (tests, `retouch_ui`, the clone
/// overlay's auto-hide). During the run the family state is out of the app.
pub(crate) fn stroking(app: &PhotocraftApp) -> Option<&PaintStroke> {
    app.canvas_tools.paint.stroke.as_ref()
}

/// Feeds the live stroke the points the pass's moves added, once per pass and after the host ran,
/// so the stroke shows them in the same pass. Only the points not yet fed are fed, so a second
/// pass of the same frame (which sees only its own moves) never repeats the first one's.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut PaintState, _response: &ToolHostResponse) {
    let Some(stroke) = state.stroke.as_ref() else { return };
    feed_live_stroke(app, stroke);
    let ctx = app.ctx().clone();
    // The stroke is still painting what it was given: show its steps as they finish.
    if live_stroke_busy(app) {
        craft::ui::runtime::frame::request_repaint(&ctx);
    }
    // A held pen keeps the clock running for airbrush build-up and smoothing catch-up (asked
    // after the feed, which brings in what the stroke's thread last painted).
    if stroke_wants_time(app) {
        craft::ui::runtime::frame::request_repaint_after(&ctx, Duration::from_millis(HOLD_REPEAT_MS as u64).as_secs_f64());
    }
}

/// The document point under screen position `pos`.
fn doc_point(cx: &ToolContext<'_>, pos: egui::Pos2) -> Option<[f64; 2]> {
    let p = cx.space()?.view.to_doc(pos);
    Some([p.x, p.y])
}

/// Ends the running stroke without a commit.
fn drop_stroke(app: &mut PhotocraftApp, state: &mut PaintState) {
    state.stroke = None;
    app.live_stroke = None;
    app.session.clear_live_stroke();
    app.trail = None;
}

/// Is a held pen's repeat point due at `now_ms`, so the stroke's time keeps advancing? Only a
/// stroke that changes while the pointer is still (airbrush, smoothing catching up) repeats. The
/// last point's time is the pass's after it repeats, so a second pass of a frame repeats nothing.
fn hold_due(app: &PhotocraftApp, now_ms: f64) -> bool {
    app.stylus.last_point_ms().is_some_and(|t| now_ms - t >= HOLD_REPEAT_MS) && stroke_wants_time(app)
}

/// Pushes a point of `stroke` at `at` with the stylus's pressure and records its pose.
fn push_point(app: &mut PhotocraftApp, stroke: &mut PaintStroke, at: [f64; 2]) {
    stroke.points.push([at[0], at[1], f64::from(app.stylus.pressure())]);
    app.stylus.record_point();
}

/// The Brush's chosen Clear paints as Normal on the Background, and its Mode then reads Normal
/// (#11897): at the press of a Brush stroke on the Background's own pixels, Clear (chosen in the
/// options bar, not a held `~`) is turned off, so the stroke is sent as Normal. A Background whose
/// pixels are locked refuses the stroke, so Clear stays chosen there.
fn clear_off_on_background(app: &mut PhotocraftApp, tool: Tool) {
    let on_background = app.session.active().is_some_and(|st| {
        st.active_layer.is_some_and(|id| {
            st.doc.is_background(id) && {
                let locks = st.doc.effective_locks(id);
                !locks.pixels && !locks.all
            }
        })
    });
    if tool == Tool::Brush && app.ui.tool_options.brush_clear && !app.held_erase && on_background && paint_target(app) == json!("pixels") {
        app.ui.tool_options.brush_clear = false;
    }
}

/// Reports why a Mixer Brush press could not start: the two refusals every press would meet get
/// one alert, anything else the status bar.
fn mixer_press_failed(app: &mut PhotocraftApp, message: String) {
    use photocraft_engine::brush_cmds::mixer_live::{BITMAP_REFUSAL, INDEXED_REFUSAL};
    if message == INDEXED_REFUSAL || message == BITMAP_REFUSAL {
        // `alert` shows one alert even if more presses arrive while it is open.
        let _ = crate::dialogs::alerts::alert(app, message);
    } else {
        app.ui.status = message;
        app.ui.status_error = true;
    }
}

/// Runs `f` on the brush resize drag with the app, taking the resize out of the app for the call.
fn with_resize<R>(app: &mut PhotocraftApp, f: impl FnOnce(&mut crate::brush_resize::PaintResize, &mut PhotocraftApp) -> R) -> R {
    let mut resize = std::mem::take(&mut app.brush_resize);
    let out = f(&mut resize, app);
    app.brush_resize = resize;
    out
}

/// A press the paint tool takes: the resize chord, the HUD Color Picker, a Mixer Brush load (a click
/// that is done at once), or a stroke.
/// A Paint Bucket or Magic Eraser press: the fill or the erase runs at the clicked pixel. Returns false for any other tool.
fn click_tool(app: &mut PhotocraftApp, tool: Tool, at: [f64; 2]) -> bool {
    match tool {
        Tool::PaintBucket => {
            let o = app.ui.tool_options.clone();
            let contents = if o.bucket_fill_pattern { "pattern" } else { "foreground" };
            let mode = if o.bucket_clear { "clear" } else { o.bucket_blend_mode.label() };
            let params = json!({
                "x": at[0].floor(),
                "y": at[1].floor(),
                "tolerance": o.tolerance,
                "contiguous": o.contiguous,
                "antiAlias": o.anti_alias,
                "sampleAllLayers": o.sample_all.paint_bucket,
                "opacity": o.fill_opacity,
                "contents": contents,
                "mode": mode,
                "target": paint_target(app),
            });
            match app.run("paint.bucket", params) {
                // A Bitmap document answers with the reference's alert (#12536), once however many clicks arrive
                // while it is open; no status-bar error as well.
                Err(e) if e == photocraft_engine::paint_cmds::BITMAP_REFUSAL => {
                    let _ = crate::dialogs::alerts::alert(app, tl!(photocraft_engine::paint_cmds::BITMAP_REFUSAL));
                }
                Err(e) => {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
                Ok(_) => {}
            }
            true
        }
        Tool::MagicEraser => crate::eraser_ui::click(app, tool, at[0], at[1]),
        _ => false,
    }
}

fn bucket_targets_pasteboard(app: &PhotocraftApp, at: [f64; 2], mods: Mods) -> bool {
    let plain_shift = mods.shift && !mods.alt && !mods.ctrl && !mods.cmd;
    let Some(st) = app.session.active() else { return false };
    let (w, h) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    let (x, y) = (at[0], at[1]);
    let outside = x.is_finite() && y.is_finite() && (x < 0.0 || y < 0.0 || x >= w || y >= h);
    plain_shift && outside && st.doc.selection.is_none()
}

/// Paint Bucket ⇧-click on the pasteboard sets the pasteboard colour to the foreground colour, as
/// in the reference application (#12534): the custom colour, and use it, as the pasteboard menu's
/// Select Custom Color… does. It is a preference, so no history state and no document change.
/// Only a plain ⇧ with no selection: other modifiers and an active selection were not measured and
/// keep today's fill. True when it handled the click.
pub(crate) fn bucket_pasteboard_click(app: &mut PhotocraftApp, at: [f64; 2], mods: Mods) -> bool {
    if !bucket_targets_pasteboard(app, at, mods) {
        return false;
    }
    let [r, g, b, _] = app.session.tools.foreground;
    let color = crate::color_picker_ui::hex([r, g, b]);
    if let Err(e) = app.run("prefs.set", json!({"values": {"interface.canvasColor": "custom", "interface.canvasCustomColor": color}})) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

fn begin(app: &mut PhotocraftApp, state: &mut PaintState, pointer: &ToolPointer, at: [f64; 2], cx: &mut ToolContext<'_>) -> ToolAction {
    let tool = app.ui.tool;
    let (mods, button) = (pointer.mods, pointer.button);
    app.stylus.record_pointer(pointer.time * 1000.0, Some(pointer.sample));
    if with_resize(app, |r, app| r.press(app, pointer, cx)) {
        return ToolAction::Claimed;
    }
    let secondary = if button == PointerButton::Secondary { secondary_takes(app, tool, mods) } else { SecondaryUse::None };
    // Shift+Alt+right: the HUD stays up until the button comes up.
    if secondary == SecondaryUse::Hud {
        crate::color_hud::open(app, [pointer.pos.x, pointer.pos.y], true);
        return ToolAction::Claimed;
    }
    // A Mixer Brush press that loads paint is a click, done at once: the host spends the press, so
    // its drags and its release reach no tool. `ctrl` is the Control key on macOS only, so the
    // Command stand-in's press (`cmd`) is excluded here too.
    if button == PointerButton::Primary && alt_loads_mixer(tool, mods) && !mods.cmd {
        let pen = app.stylus.sample().unwrap_or_default();
        let pressure = app.stylus.pressure();
        // Load Solid Colors Only loads the one colour under the hotspot (#11952).
        let (id, p) = if app.session.tools.brush.mixer.solid_colors_only {
            ("paint.mixerLoad", json!({"x": at[0], "y": at[1]}))
        } else {
            ("brush.mixer.sample", json!({"points": [[at[0], at[1], pressure, pen.tilt_x, pen.tilt_y, pen.rotation]], "target": paint_target(app)}))
        };
        if let Err(e) = app.run(id, p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return ToolAction::Done;
    }
    // The Paint Bucket and the Magic Eraser act on the click: no stroke, no drag.
    if button == PointerButton::Primary && click_tool(app, tool, at) {
        return ToolAction::Done;
    }
    sync_tool_brush(app);
    clear_off_on_background(app, tool);
    // Clear mode (`~` held, or chosen in the options bar) erases with the tool's own brush.
    let clear = clears(app, tool);
    let erase = secondary == SecondaryUse::Erase || tool == Tool::Eraser || clear;
    // A 1 px pencil line depends on the pixels, not where inside the pixel the click landed
    // (#4071). Snap both ends of the stroke to the aliased tip's grid.
    let (x, y) = snap_aliased_tip(app, tool, at[0], at[1]);
    // ⇧-click connects to the last endpoint, including its pen pressure (#2827), so the renderer
    // interpolates size and opacity across the segment.
    let active = app.session.active().map(|st| st.doc.id);
    let from = app.last_stroke_end.filter(|(doc, _)| mods.shift && crate::stroke_constraint::connects(tool) && Some(*doc) == active).map(|(_, p)| {
        let (sx, sy) = snap_aliased_tip(app, tool, p[0], p[1]);
        [sx, sy, p[2]]
    });
    let mut points = vec![[x, y, f64::from(app.stylus.pressure())]];
    if let Some(p) = from {
        points.insert(0, p);
    }
    // ⌥ flips Dodge/Burn and Blur/Sharpen for this stroke.
    let tool = crate::retouch_ui::alt_flipped(tool, mods.alt);
    // Erase to History (or ⌥ for this stroke): the Eraser's brush stroke paints the History
    // panel's source back.
    let tool = crate::eraser_ui::drag_tool(app, tool, mods.alt);
    let mut stroke = PaintStroke::new(tool, from.map_or([x, y], |p| [p[0], p[1]]), points, mods, erase);
    stroke.clear = clear;
    stroke.line = from.is_some();
    stroke.history_params = crate::retouch_ui::history_params_at_press(app, tool, mods.alt);

    app.trail = None;
    app.stylus.begin_stroke();
    if from.is_some() {
        app.stylus.record_point();
    }
    app.live_stroke = if strokes_live(tool) { begin_live_stroke(app, &stroke) } else { None };
    if tool == Tool::MixerBrush && app.live_stroke.is_none() {
        use photocraft_engine::brush_cmds::mixer_live::LiveMixer;
        let p = stroke_params(app, &stroke, &app.stylus.stroke_points(&stroke.points));
        // A tip above the preview size keeps the stroke: the release renders it. Otherwise the
        // first dab itself is invalid; do not queue an unexpected release edit.
        if !LiveMixer::defers_to_release(&app.session, &p).is_ok_and(|deferred| deferred) {
            if let Err(e) = LiveMixer::begin(&app.session, &p) {
                mixer_press_failed(app, e.to_string());
            }
            return ToolAction::Done;
        }
    } else if tool == Tool::MixerBrush {
        feed_live_stroke(app, &stroke);
    }
    state.stroke = Some(stroke);
    ToolAction::Claimed
}

impl CanvasTool for PaintTool<'_> {
    /// Keeps the brush tip cursor in step with the brush and repeats a held pen's point. A Mixer
    /// stroke whose document or revision moved on is dropped (its uncommitted paint belongs to the
    /// old picture).
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        let Self { scope, state } = self;
        let now_ms = cx.time * 1000.0;
        scope.with_app(|app| {
            crate::brush_cursor::sync_tip(app);
            // The readout request lasts one pass: a pass without pointer input repeats it.
            app.brush_resize.hud(cx);
            let Some(stroke) = state.stroke.as_ref() else { return };
            let stale_mixer =
                stroke.tool == Tool::MixerBrush && app.live_stroke.is_some() && app.session.active_index().is_none_or(|idx| live_stroke(app, idx).is_none());
            if stale_mixer {
                drop_stroke(app, state);
                return;
            }
            if hold_due(app, now_ms) {
                let Some(stroke) = state.stroke.as_mut() else { return };
                let at = stroke.points.last().map_or(stroke.start, |p| [p[0], p[1]]);
                app.stylus.repeat_point(now_ms);
                stroke.points.push([at[0], at[1], f64::from(app.stylus.pressure())]);
            }
        });
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(at) = doc_point(cx, pointer.pos) else { return ToolAction::Ignored };
        let Self { scope, state } = self;
        scope.with_app(|app| begin(app, state, pointer, at, cx))
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Self { scope, state } = self;
        scope.with_app(|app| {
            if with_resize(app, |r, app| {
                let active = r.active();
                if active {
                    r.drag(app, pointer, cx);
                }
                active
            }) {
                return;
            }
            let Some(at) = doc_point(cx, pointer.pos) else { return };
            let Some(stroke) = state.stroke.as_mut() else { return };
            let now_ms = pointer.time * 1000.0;
            app.stylus.record_pointer(now_ms, Some(pointer.sample));
            stroke.track(pointer.mods);
            // ⇧: straight 0/45/90° strokes (stroke_constraint.rs).
            let last = stroke.points.last().map_or(stroke.start, |p| [p[0], p[1]]);
            let zoom = app.point_zoom();
            let [x, y] = crate::stroke_constraint::constrain(stroke.tool, &mut stroke.constrain, stroke.start, last, at, pointer.mods.shift, zoom);
            let moved = stroke.points.last().is_none_or(|p| (p[0] - x).abs() + (p[1] - y).abs() > 0.25);
            if moved || hold_due(app, now_ms) {
                push_point(app, stroke, [x, y]);
            }
            // The canvas shows a live stroke itself; a stroke without one (a retouch tool that
            // paints on release, a live stroke that could not start) shows its footprint trail.
            if !(strokes_live(stroke.tool) && app.live_stroke.is_some()) {
                crate::stroke_trail::feed(app, stroke.tool, &stroke.points);
            }
        });
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let at = doc_point(cx, pointer.pos);
        let Self { scope, state } = self;
        scope.with_app(|app| {
            if with_resize(app, |r, app| {
                let active = r.active();
                if active {
                    r.release(app, pointer, cx);
                }
                active
            }) {
                return;
            }
            let Some(mut stroke) = state.stroke.take() else {
                // The HUD Color Picker gesture: a control-channel one closes here.
                crate::color_hud::release(&mut app.ui);
                return;
            };
            app.stylus.record_pointer(pointer.time * 1000.0, Some(pointer.sample));
            stroke.track(pointer.mods);
            // A pen tap paints at its peak pressure; a mouse is unchanged.
            tap_at_peak_pressure(app, &mut stroke);
            if let Some(at) = at {
                let last = stroke.points.last().map_or(stroke.start, |p| [p[0], p[1]]);
                let zoom = app.point_zoom();
                let [x, y] = crate::stroke_constraint::constrain(stroke.tool, &mut stroke.constrain, stroke.start, last, at, pointer.mods.shift, zoom);
                // The release point snaps like the press did, so a click that never moved adds no
                // second point: it paints its one dab.
                let (x, y) = snap_aliased_tip(app, stroke.tool, x, y);
                if stroke.points.last().is_none_or(|p| p[0] != x || p[1] != y) {
                    let pressure = stroke.points.last().map_or(1.0, |p| p[2]);
                    stroke.points.push([x, y, pressure]);
                    app.stylus.record_point();
                }
            }
            super::strokes::finish(app, stroke);
            app.trail = None;
        });
    }

    fn cancel(&mut self, _reason: CancelReason, cx: &mut ToolContext<'_>) {
        let Self { scope, state } = self;
        scope.with_app(|app| {
            drop_stroke(app, state);
            with_resize(app, |r, app| r.cancel(app, cx));
            // A cancelled right-press (Escape) ends the HUD whoever opened it; the held button's release is then no gesture.
            crate::color_hud::close(&mut app.ui);
        });
    }

    fn claims(&self) -> ToolClaims {
        if self.scope.with_app(|app| crate::retouch_ui::bar::pending(app, Tool::Remove).is_some()) {
            ToolClaims::NONE.keys(PENDING_KEYS)
        } else {
            ToolClaims::NONE
        }
    }

    fn key(&mut self, key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        self.scope.with_app(|app| {
            if crate::retouch_ui::bar::pending(app, Tool::Remove).is_none() {
                return false;
            }
            match key {
                ToolKey::Submit => crate::retouch_ui::remove_pending_commit(app),
                ToolKey::Cancel => crate::retouch_ui::remove_pending_cancel(app),
                ToolKey::Kept(chord) if *chord == PENDING_KEYS[0] => crate::retouch_ui::remove_pending_commit(app),
                ToolKey::Kept(chord) if *chord == PENDING_KEYS[1] => crate::retouch_ui::remove_pending_cancel(app),
                _ => false,
            }
        })
    }

    /// The feedback of a painting tool (the smoothing string, the clone overlay, the stroke trail)
    /// is a canvas control `brush_feedback::show` puts in the canvas body.
    fn paint(&self, _draw: &Draw<'_>, _cx: &ToolContext<'_>) {}

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        self.scope.with_app(|app| {
            if app.brush_resize.active() {
                return Cursor::Painted(PaintedCursor::Hidden);
            }
            let tool = app.ui.tool;
            // ⌥ on the Mixer Brush loads paint: its sample target, or the crosshair for precise cursors.
            if alt_loads_mixer(tool, cx.mods) && !cx.mods.cmd {
                return if cx.precise() { Cursor::Painted(PaintedCursor::Crosshair) } else { sample_target() };
            }
            let sets_pasteboard = tool == Tool::PaintBucket
                && cx.mods.shift
                && cx.pointer.as_ref().and_then(|pointer| doc_point(cx, pointer.pos)).is_some_and(|at| bucket_targets_pasteboard(app, at, cx.mods));
            if !sets_pasteboard && paint_cursor_blocked(app) {
                return Cursor::System(CursorIcon::NotAllowed);
            }
            // The Paint Bucket and the Magic Eraser act on a click: their own pointers, no brush tip.
            if let Some(cursor) = click_tool_cursor(tool, cx.precise()) {
                return cursor;
            }
            crate::brush_cursor::cursor(app, cx)
        })
    }

    /// The paint tool takes the primary button, the secondary one for the erase, resize and HUD
    /// gestures, and a Mixer Brush ⌥ load. The Command stand-in (Move) and the Alt stand-ins
    /// (the Eyedropper, the clone source) take their presses first: this tool refuses them.
    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        self.scope.with_app(|app| {
            let tool = app.ui.tool;
            match button {
                PointerButton::Primary => {
                    if crate::brush_resize::PaintResize::chord(tool, button, mods) || (alt_loads_mixer(tool, mods) && !mods.cmd) {
                        return true;
                    }
                    // Clone Stamp and Healing take ⌥ as the source stand-in (`retouch::source_tool`).
                    let source_alt = mods.alt && matches!(tool, Tool::CloneStamp | Tool::Healing);
                    !(mods.cmd || alt_samples(tool, mods) || source_alt)
                }
                PointerButton::Secondary => secondary_takes(app, tool, mods) != SecondaryUse::None,
                _ => false,
            }
        })
    }

    fn takes_every_move(&self) -> bool {
        true
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("painting is by pointer only; Edit > Fill and Edit > Stroke fill and outline a selection by command")
    }
}

#[cfg(test)]
mod tests;
