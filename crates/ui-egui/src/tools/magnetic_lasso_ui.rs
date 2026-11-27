//! Magnetic Lasso Tool (L): the app side of the framework's `MagneticLassoTool`.
//!
//! The tool (pointer rules, fastening, close, retract, width keys, cursor and outline paint) lives in
//! `craft::imaging::ui::select` and is held by `SelectionTools::magnetic`. This module supplies what
//! only the app knows:
//!
//! - [`PcEdges`], the edge finder over the document's pixels (the engine's `EdgeSource`);
//! - [`prepare`] and [`store`], which carry the options bar's settings into the tool and back around
//!   each pass;
//! - [`commit`], which selects the closed border through `select.magneticLasso` with
//!   `"trace": false`, so the selection is exactly the outline drawn, in one history step;
//! - [`inspect`], the border in progress for `ui.inspect`.
//!
//! The border follows the edges of the visible composite, whichever layer is active; documents that
//! are not RGB keep following the active layer's own pixels (`photocraft_engine::magnetic_cmds`).

use craft::behavior::kurbo::Point;
use craft::imaging::ui::select::{EdgeSearch, EdgeTracer, MagneticLassoTool};
use photocraft_algo::magnetic::Settings;
use photocraft_engine::Session;
use photocraft_engine::magnetic_cmds::EdgeSource;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::tool_feedback::mode_name;

/// The edge finder over the pixels being traced: a snapshot of the document, rebuilt when the
/// engine says it is no longer current.
#[derive(Default)]
pub(crate) struct PcEdges {
    source: Option<EdgeSource>,
}

impl PcEdges {
    /// Rebuilds the snapshot when the document, the layer or its revision changed
    /// (`EdgeSource::is_current`, the engine's own signal).
    pub(crate) fn sync(&mut self, session: &Session) {
        if !self.source.as_ref().is_some_and(|s| s.is_current(session)) {
            self.source = EdgeSource::new(session);
        }
    }
}

/// The engine's detection settings for one query; `EdgeSearch::contrast` is already 0..=1.
fn settings(search: EdgeSearch) -> Settings {
    Settings::new(search.width, search.contrast as f32)
}

impl EdgeTracer for PcEdges {
    fn snap(&mut self, at: Point, search: EdgeSearch) -> Option<Point> {
        let p = self.source.as_mut()?.snap([at.x, at.y], settings(search))?;
        Some(Point::new(p[0], p[1]))
    }

    fn trace(&mut self, from: Point, to: Point, guide: &[Point], search: EdgeSearch) -> Vec<Point> {
        let Some(source) = self.source.as_mut() else { return Vec::new() };
        let guide: Vec<[f64; 2]> = guide.iter().map(|p| [p.x, p.y]).collect();
        source.trace([from.x, from.y], [to.x, to.y], &guide, settings(search)).into_iter().map(|p| Point::new(p[0], p[1])).collect()
    }
}

/// Gives the tool this pass's app state, before the host runs it: the edge snapshot, the options
/// bar's settings.
pub(crate) fn prepare(app: &PhotocraftApp, tool: &mut MagneticLassoTool<PcEdges>) {
    tool.tracer_mut().sync(&app.session);
    *tool.settings_mut() = app.ui.tool_options.magnetic;
}

/// Writes the tool's settings back after the run, so `[` and `]` reach the options store.
pub(crate) fn store(app: &mut PhotocraftApp, tool: &MagneticLassoTool<PcEdges>) {
    app.ui.tool_options.magnetic = *tool.settings();
}

/// Selects the closed border as drawn, in `mode` (the family's operation-to-mode name). Fewer than
/// three points select nothing; an error goes to the status line.
pub(crate) fn commit(app: &mut PhotocraftApp, points: &[Point], mode: &str) {
    if points.len() < 3 {
        return;
    }
    let path: Vec<[f64; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
    let o = &app.ui.tool_options;
    let params = json!({"points": path, "trace": false, "mode": mode, "antiAlias": o.anti_alias, "feather": o.feather});
    if let Err(e) = app.run("select.magneticLasso", params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The border in progress for `ui.inspect`: `path`, `anchors` (indices into `path`), `live` and the
/// selection `mode`; empty and `""` when no border is open.
pub(crate) fn inspect(app: &PhotocraftApp) -> Value {
    let Some(o) = app.canvas_tools.selection.magnetic.outline() else {
        return json!({"path": [], "anchors": [], "live": [], "mode": ""});
    };
    let pts = |v: &[Point]| v.iter().map(|p| json!([p.x, p.y])).collect::<Vec<Value>>();
    json!({"path": pts(o.path), "anchors": o.anchors, "live": pts(o.live), "mode": mode_name(o.operation)})
}

#[cfg(test)]
mod tests;
