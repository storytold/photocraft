//! The Object Selection tool's SAM 3.1 mode (ShotoPhopEX): with a generative server that
//! answers, the tool works the way the detector wants to be used.
//!
//! - **Object finder**: when the pointer rests over the picture the detector is asked what a
//!   click there would select (`select.probe`, a quiet background job that selects nothing) and
//!   the answer tints the object under the pointer, as Photoshop's Object Finder does. The click
//!   that follows applies that probe without another round trip (`select.byPoint {probe}`).
//! - **Click** selects the object under the pointer; **drag** a box around an object and the
//!   detector finds it inside the box (`select.byPoint {box}`); ⇧ adds the object to the
//!   selection and ⌥ subtracts it, as the classical tool does.
//! - **Refine** (the options bar's second mode; Ctrl is the Move tool's, as in Photoshop): a
//!   click adds a spot the object must include, ⌥-click one it must not. The last query runs
//!   again with every point so far, replacing its own result (one undo step comes back, one goes
//!   in), so the mask grows or shrinks instead of accumulating separate detections.
//! - **Refine edge** (the options bar) softens the detector's hard edge with the classical
//!   refinement, as Remove Background does.
//!
//! Everything is engine commands; the UI keeps the pointer's cell, the pending probe, the mask
//! texture and the last query's points. Without a server, or with one that is down, the tool
//! stays the classical rectangle finder.

use base64::Engine as _;
use egui::{Color32, Rect, pos2};
use photocraft_doc::DocId;
use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::retouch_ui::{server_configured, server_online};
use crate::state::Tool;

/// Seconds the pointer rests before the finder asks.
pub const PROBE_DELAY: f64 = 0.25;
/// Pointer positions this close (document pixels) count as the same spot.
const CELL: f64 = 6.0;
/// How often an answering server is re-checked (seconds).
const ONLINE_RECHECK: f64 = 10.0;
/// How strongly the finder tints the object.
const TINT_ALPHA: f32 = 0.45;

/// A probe's answer, ready to draw.
#[derive(Clone)]
pub struct Probe {
    pub cell: [i64; 2],
    /// The engine's probe id (`select.byPoint {probe}` applies it).
    pub id: u64,
    pub doc: DocId,
    /// Canvas pixels.
    pub bounds: [i32; 4],
    tex: egui::TextureHandle,
}

impl PartialEq for Probe {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id && self.doc == o.doc
    }
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Probe({} at {:?})", self.id, self.cell)
    }
}

/// The last detection the tool ran, for Ctrl-click refinement.
#[derive(Clone, Debug, PartialEq)]
pub struct LastQuery {
    pub doc: DocId,
    pub positives: Vec<[f64; 2]>,
    pub negatives: Vec<[f64; 2]>,
    pub bbox: Option<[f64; 4]>,
    pub mode: String,
    /// The document's history length after it ran (another edit since ends the refinement).
    pub past_len: usize,
}

/// The tool's state in `UiState::object_select`. Never saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectSelectUi {
    /// Where the pointer rests: its cell and since when (egui time).
    hover: Option<([i64; 2], f64)>,
    /// The probe job in flight and the cell it is for.
    pending: Option<(u64, [i64; 2])>,
    /// A probe answer waiting for a frame to become a texture.
    incoming: Option<([i64; 2], Value)>,
    pub probe: Option<Probe>,
    pub last: Option<LastQuery>,
    /// The last health check: the server asked, answering, and when (egui time).
    online: Option<(String, bool, f64)>,
    /// Probes asked this session (inspect, tests).
    pub probes_asked: u32,
}

fn cell_of(p: [f64; 2]) -> [i64; 2] {
    [(p[0] / CELL).floor() as i64, (p[1] / CELL).floor() as i64]
}

/// Seconds the pointer rests before the finder asks: shorter with the native engine, whose
/// worker keeps the picture's embedding and answers a probe in tens of milliseconds.
pub fn probe_delay(app: &PhotocraftApp) -> f64 {
    if crate::retouch_ui::native_engine(app) { 0.08 } else { PROBE_DELAY }
}

/// Is the server answering? Checked at most every [`ONLINE_RECHECK`] seconds (a health request).
pub fn online(app: &mut PhotocraftApp, now: f64) -> bool {
    if !server_configured(app) {
        app.ui.object_select.online = None;
        return false;
    }
    // The answer is for one configuration: the engine and whatever it needs to be reached.
    let i = &app.session.prefs().integrations;
    let url = format!("{}|{}", i.runtime_folders(), i.models_root());
    if let Some((u, ok, at)) = &app.ui.object_select.online
        && *u == url
        && now - at < ONLINE_RECHECK
        && now >= *at
    {
        return *ok;
    }
    let ok = server_online(app);
    app.ui.object_select.online = Some((url, ok, now));
    ok
}

/// Forget the finder's answer and anything in flight (the tool, document or picture changed).
pub fn reset(app: &mut PhotocraftApp) {
    if let Some((job, _)) = app.ui.object_select.pending.take() {
        crate::jobs_ui::cancel(app, JobId(job));
    }
    let o = &mut app.ui.object_select;
    o.hover = None;
    o.incoming = None;
    o.probe = None;
}

/// Each frame, after the canvas knows where the pointer is: run the finder for a resting
/// pointer and turn its answer into the texture the canvas draws.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let active_doc = app.session.active().map(|d| d.doc.id);
    let wanted = app.ui.tool == Tool::ObjectSelection && app.ui.tool_options.object_finder && app.drag.is_none() && active_doc.is_some();
    if !wanted {
        if app.ui.object_select.probe.is_some() || app.ui.object_select.pending.is_some() || app.ui.object_select.hover.is_some() {
            reset(app);
        }
        return;
    }
    // A probe of another document is gone with it.
    if app.ui.object_select.probe.as_ref().is_some_and(|p| Some(p.doc) != active_doc) {
        app.ui.object_select.probe = None;
    }
    // An answer that arrived: decode its mask into the tint texture.
    if let Some((cell, v)) = app.ui.object_select.incoming.take()
        && let (Some(doc), Some(tex)) = (active_doc, texture_of(ctx, &v, crate::theme::Tokens::get(ctx).accent))
    {
        let b = v["bounds"].as_array().map(|a| a.iter().filter_map(Value::as_i64).map(|x| x as i32).collect::<Vec<_>>()).unwrap_or_default();
        let bounds = [b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0), b.get(2).copied().unwrap_or(0), b.get(3).copied().unwrap_or(0)];
        app.ui.object_select.probe = Some(Probe { cell, id: v["probe"].as_u64().unwrap_or(0), doc, bounds, tex });
        ctx.request_repaint();
    }
    let inside = app.hover_doc.filter(|p| {
        app.session.active().is_some_and(|d| p[0] >= 0.0 && p[1] >= 0.0 && p[0] < f64::from(d.doc.size.width) && p[1] < f64::from(d.doc.size.height))
    });
    let Some(p) = inside else {
        app.ui.object_select.hover = None;
        return;
    };
    let cell = cell_of(p);
    let delay = probe_delay(app);
    match app.ui.object_select.hover {
        Some((c, _)) if c == cell => {}
        _ => {
            app.ui.object_select.hover = Some((cell, now));
            ctx.request_repaint_after(std::time::Duration::from_millis((delay * 1000.0) as u64 + 10));
            return;
        }
    }
    let rested = app.ui.object_select.hover.is_some_and(|(_, since)| now - since >= delay);
    let known = app.ui.object_select.probe.as_ref().is_some_and(|pr| pr.cell == cell);
    if !rested || known {
        return;
    }
    // One probe in flight at a time; a pointer that moved on makes the old one worthless.
    if let Some((job, c)) = app.ui.object_select.pending {
        if c == cell {
            return;
        }
        app.ui.object_select.pending = None;
        crate::jobs_ui::cancel(app, JobId(job));
    }
    if !online(app, now) {
        return;
    }
    let sample_all = app.ui.tool_options.sample_all.object_selection;
    app.ui.object_select.probes_asked += 1;
    match app.run("select.probe", json!({"x": p[0].round(), "y": p[1].round(), "sampleAllLayers": sample_all})) {
        Ok(v) if v.get("job").is_some() => app.ui.object_select.pending = v["job"].as_u64().map(|j| (j, cell)),
        // Inline (tests, no background jobs): the answer is here already.
        Ok(v) => app.ui.object_select.incoming = Some((cell, v)),
        Err(_) => {
            // Nothing under the pointer (or a server hiccup): remember the cell so the pointer
            // has to move before the finder asks again.
            app.ui.object_select.probe = None;
            app.ui.object_select.hover = Some((cell, f64::INFINITY));
        }
    }
}

/// A probe job ended (`jobs_ui::on_event`).
pub fn on_event(app: &mut PhotocraftApp, e: &JobEvent) {
    if e.command != "select.probe" {
        return;
    }
    let Some((job, cell)) = app.ui.object_select.pending else { return };
    if job != e.id.0 {
        return;
    }
    app.ui.object_select.pending = None;
    match &e.outcome {
        JobOutcome::Done(v) => app.ui.object_select.incoming = Some((cell, v.clone())),
        _ => {
            app.ui.object_select.probe = None;
            app.ui.object_select.hover = Some((cell, f64::INFINITY));
        }
    }
}

/// The probe's mask as a texture: the accent colour where the object is.
fn texture_of(ctx: &egui::Context, v: &Value, accent: Color32) -> Option<egui::TextureHandle> {
    let (w, h) = (v["width"].as_u64()? as usize, v["height"].as_u64()? as usize);
    let data = base64::engine::general_purpose::STANDARD.decode(v["mask"].as_str()?).ok()?;
    if w == 0 || h == 0 || data.len() != w * h {
        return None;
    }
    let [r, g, b, _] = accent.to_array();
    let px: Vec<Color32> = data.iter().map(|m| Color32::from_rgba_unmultiplied(r, g, b, (f32::from(*m) * TINT_ALPHA) as u8)).collect();
    Some(ctx.load_texture("object-finder", egui::ColorImage::new([w, h], px), egui::TextureOptions::LINEAR))
}

/// Draw the finder's tint over the object under the pointer (the canvas, before the ants).
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if app.ui.tool != Tool::ObjectSelection || app.drag.is_some() {
        return;
    }
    let Some(pr) = app.ui.object_select.probe.as_ref() else { return };
    let Some(st) = app.session.active().filter(|d| d.doc.id == pr.doc) else { return };
    let rect = xf.doc_rect(st.doc.bounds());
    painter.image(pr.tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

/// Object Selection's modifiers: ⌥ subtracts, ⇧ adds (see `tool_feedback::selection_mode`).
fn object_mode(mods: egui::Modifiers) -> &'static str {
    if mods.alt {
        "subtract"
    } else if mods.shift {
        "add"
    } else {
        "replace"
    }
}

/// The gesture ended: a click, a box, or a Ctrl-click refinement; the classical rectangle when
/// no server answers.
/// Without a server: upstream's classical tool. A click (or a sub-2 px nudge) selects the
/// object under it when the Object Finder is on, as hovering highlighted it (#4087; ⇧ adds,
/// ⌥ subtracts, ⇧⌥ intersects); with the finder off it is a Quick Selection dab at the brush
/// size instead of looking dead (⌥ subtracts). A larger drag runs the rectangle GrabCut (⌥
/// subtracts, ⇧ adds, otherwise replaces).
fn classical_finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers, sample_all: bool) {
    let (x, y) = (start[0].min(end[0]), start[1].min(end[1]));
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let click = w < 2.0 || h < 2.0;
    if click && app.ui.tool_options.object_finder {
        let mode = match (mods.shift, mods.alt) {
            (true, true) => "intersect",
            (true, false) => "add",
            (false, true) => "subtract",
            (false, false) => "replace",
        };
        let r = app.run("select.object", json!({"point": [end[0], end[1]], "mode": mode, "sampleAllLayers": sample_all}));
        crate::object_finder::clear(app);
        crate::retouch_ui::report_smart_result(app, r, tl!("No object found there — drag a rectangle around it"));
        return;
    }
    if click {
        let size = app.session.tools.brush.size;
        let mode = if mods.alt { "subtract" } else { "add" };
        let r = app.run("select.quick", json!({"points": [[end[0], end[1]]], "size": size, "mode": mode, "sampleAllLayers": sample_all}));
        crate::retouch_ui::report_smart_result(app, r, tl!("Quick Selection found nothing there"));
        return;
    }
    let mode = if mods.alt {
        "subtract"
    } else if mods.shift {
        "add"
    } else {
        "replace"
    };
    let r = app.run("select.object", json!({"rect": [x.round(), y.round(), w.round(), h.round()], "mode": mode, "sampleAllLayers": sample_all}));
    crate::retouch_ui::report_smart_result(app, r, tl!("Object Selection found nothing there — try a tighter rectangle"));
}

/// The options bar's mask stack (Widen, Fill Holes, Soften) on a detector query: only what is
/// set goes along, so a plain click sends what it always did.
pub fn mask_stack(app: &PhotocraftApp, q: &mut Value) {
    let o = &app.ui.tool_options;
    if o.object_widen != 0.0 {
        q["grow"] = json!(o.object_widen);
    }
    if o.object_fill_holes {
        q["fillHoles"] = json!(true);
    }
    if o.object_soften > 0.0 {
        q["soften"] = json!(o.object_soften);
    }
}

pub fn finish(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: egui::Modifiers) {
    let (x, y) = (start[0].min(end[0]), start[1].min(end[1]));
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let click = w < 2.0 || h < 2.0;
    let mode = object_mode(mods);
    let o = &app.ui.tool_options;
    let (sample_all, refine) = (o.sample_all.object_selection, o.refine_edge);
    let now = app.ui.object_select.hover.map_or(0.0, |(_, t)| t);
    if !online(app, now) {
        classical_finish(app, start, end, mods, sample_all);
        return;
    }
    let point = [start[0].round(), start[1].round()];
    // Refine mode: a click adds a spot to the last object, ⌥-click excludes one (a new object
    // when there is nothing to refine).
    if click && app.ui.tool_options.object_refine && refine_last(app, point, !mods.alt) {
        return;
    }
    let mut q = json!({"mode": mode, "sampleAllLayers": sample_all, "refine": refine});
    mask_stack(app, &mut q);
    let (positives, bbox) = if click {
        q["points"] = json!([point]);
        // The finder's answer for this very spot applies at once.
        if let Some(pr) = app.ui.object_select.probe.as_ref().filter(|pr| pr.cell == cell_of(start)) {
            q["probe"] = json!(pr.id);
        }
        (vec![point], None)
    } else {
        let b = [x.round(), y.round(), w.round(), h.round()];
        q["box"] = json!(b);
        (Vec::new(), Some(b))
    };
    let doc = app.session.active().map(|d| d.doc.id);
    match app.run("select.byPoint", q) {
        Ok(_) => {
            if let (Some(doc), Some(past_len)) = (doc, app.session.active().map(|d| d.history.past_len())) {
                app.ui.object_select.last = Some(LastQuery { doc, positives, negatives: Vec::new(), bbox, mode: mode.to_string(), past_len });
            }
            // The picture's selection changed under the probe; the next pause asks again.
            app.ui.object_select.probe = None;
            app.ui.object_select.hover = None;
        }
        Err(e) => crate::notices::error(app, e),
    }
}

/// Run the last query again with `point` added (excluded, or included with `include`),
/// replacing its result. False when there is nothing to refine (then the click is a new one).
fn refine_last(app: &mut PhotocraftApp, point: [f64; 2], include: bool) -> bool {
    let Some(last) = app.ui.object_select.last.clone() else { return false };
    let current = app.session.active().map(|d| (d.doc.id, d.history.past_len()));
    if current != Some((last.doc, last.past_len)) {
        return false;
    }
    // Take the last result back, then ask again with every point so far.
    if app.run("edit.undo", json!({})).is_err() {
        return false;
    }
    let mut next = last.clone();
    if include {
        next.positives.push(point);
    } else {
        next.negatives.push(point);
    }
    let o = &app.ui.tool_options;
    let mut q = json!({"mode": next.mode, "sampleAllLayers": o.sample_all.object_selection, "refine": o.refine_edge, "negativePoints": next.negatives});
    mask_stack(app, &mut q);
    if !next.positives.is_empty() {
        q["points"] = json!(next.positives);
    }
    if let Some(b) = next.bbox {
        q["box"] = json!(b);
    }
    match app.run("select.byPoint", q) {
        Ok(_) => {
            next.past_len = app.session.active().map_or(0, |d| d.history.past_len());
            app.ui.object_select.last = Some(next);
            app.ui.object_select.probe = None;
            app.ui.object_select.hover = None;
        }
        Err(e) => crate::notices::error(app, e),
    }
    true
}

/// `ui.inspect.objectSelect`.
pub fn inspect(app: &PhotocraftApp) -> Value {
    let o = &app.ui.object_select;
    json!({
        "probe": o.probe.as_ref().map(|p| json!({"id": p.id, "cell": p.cell, "bounds": p.bounds})),
        "pending": o.pending.map(|(j, c)| json!({"job": j, "cell": c})),
        "probesAsked": o.probes_asked,
        "last": o.last.as_ref().map(|l| json!({"positives": l.positives, "negatives": l.negatives, "box": l.bbox, "mode": l.mode})),
        "online": o.online.as_ref().map(|(_, ok, _)| *ok),
        "refineMode": app.ui.tool_options.object_refine,
    })
}

#[cfg(test)]
#[path = "object_select_tests.rs"]
mod tests;
