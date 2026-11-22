//! Each painting tool's starting tip (#11810, #11911, #12099, #12548, #12601, #12671, #12684,
//! #13029, #11936, #12200, #12964): Reset Tool and Reset All Tools (with the tool active, or picked later) give it
//! `paint_mouse::fresh_brush`, and its first use follows the tool's own rule. Other tools keep
//! their brushes.

use photocraft_engine::BrushSettings;
use serde_json::json;

use crate::PhotocraftApp;
use crate::control::{ControlRequest, handle};
use crate::paint_mouse::fresh_brush;
use crate::state::Tool;

/// What a tool's first use takes from the brush the previous tool left.
#[derive(Clone, Copy, Debug)]
enum FirstUse {
    /// Its own starting brush (`fresh_brush`); the Pencil and the Eraser keep the previous
    /// opacity (and the Eraser its flow), which `Tip` leaves out.
    Fresh,
    /// Its starting size; everything else from the previous brush.
    SizeOnly,
    /// The previous brush, at the first-use smoothing.
    Current,
}

/// The painting tools with their first-use rule; `None` for the Brush, which owns the brush the
/// app starts with, so it has no first use.
const ROWS: [(Tool, Option<FirstUse>); 16] = [
    (Tool::Burn, Some(FirstUse::Fresh)),
    (Tool::Dodge, Some(FirstUse::Fresh)),
    (Tool::Sponge, Some(FirstUse::Fresh)),
    (Tool::Blur, Some(FirstUse::Current)),
    (Tool::Sharpen, Some(FirstUse::Current)),
    (Tool::CloneStamp, Some(FirstUse::Fresh)),
    (Tool::Brush, None),
    (Tool::BackgroundEraser, Some(FirstUse::Fresh)),
    (Tool::Healing, Some(FirstUse::Fresh)),
    (Tool::HistoryBrush, Some(FirstUse::Current)),
    (Tool::PatternStamp, Some(FirstUse::Current)),
    (Tool::Remove, Some(FirstUse::SizeOnly)),
    (Tool::Pencil, Some(FirstUse::Fresh)),
    (Tool::Eraser, Some(FirstUse::Fresh)),
    (Tool::Smudge, Some(FirstUse::Fresh)),
    (Tool::SpotHealing, Some(FirstUse::Current)),
];

/// The fields of a brush that make up its tip and that the starting tips set.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tip {
    size: f64,
    hardness: f64,
    spacing: f64,
    angle: f64,
    roundness: f64,
    smoothing: f64,
    pressure_size: bool,
}

fn tip(b: &BrushSettings) -> Tip {
    Tip {
        size: f64::from(b.size),
        hardness: f64::from(b.hardness),
        spacing: f64::from(b.spacing),
        angle: f64::from(b.angle),
        roundness: f64::from(b.roundness),
        smoothing: f64::from(b.smoothing.amount),
        pressure_size: b.pressure_size,
    }
}

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// Run a menu command as the control channel does, then the frame's sync.
fn invoke(app: &mut PhotocraftApp, id: &str, params: serde_json::Value) {
    let ctx = app.ctx().clone();
    let (req, _rx) = ControlRequest::new("ui.menu.invoke", json!({"id": id, "params": params}));
    handle(app, &ctx, &req);
    app.sync_views();
}

/// Pick `tool`, and swap its brush in as a frame does.
fn pick(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(app);
    app.sync_views();
}

/// Give the live brush a tip that is no tool's starting tip, so a reset has something to undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness, b.smoothing.amount, b.pressure_size) = (57.0, 0.4, 0.6, 30.0, 0.5, 0.3, true);
}

fn live(app: &PhotocraftApp) -> Tip {
    tip(&app.session.tools.brush)
}

/// A painting tool other than `tool`, to leave it for and come back from.
fn other(tool: Tool) -> Tool {
    if tool == Tool::Pencil { Tool::Blur } else { Tool::Pencil }
}

/// The tip `tool` starts with, given the live brush.
fn starting(app: &PhotocraftApp, tool: Tool) -> Tip {
    tip(&fresh_brush(tool, &app.session.tools.brush))
}

#[test]
fn each_tool_resets_and_first_uses_its_own_tip() {
    for (tool, first_use) in ROWS {
        // Reset Tool.
        let mut a = app();
        pick(&mut a, tool);
        scramble(&mut a);
        assert_ne!(live(&a), starting(&a, tool), "{tool:?}: the scrambled tip is not its starting tip");
        invoke(&mut a, "tool.reset", json!({}));
        assert_eq!(live(&a), starting(&a, tool), "{tool:?}: Reset Tool");

        // Reset All Tools with the tool active, then with it picked later.
        let mut a = app();
        pick(&mut a, tool);
        scramble(&mut a);
        invoke(&mut a, "tool.resetAll", json!({"confirmed": true}));
        assert_eq!(live(&a), starting(&a, tool), "{tool:?}: Reset All Tools, active");
        let mut a = app();
        pick(&mut a, tool);
        scramble(&mut a);
        pick(&mut a, other(tool));
        invoke(&mut a, "tool.resetAll", json!({"confirmed": true}));
        pick(&mut a, tool);
        assert_eq!(live(&a), starting(&a, tool), "{tool:?}: Reset All Tools, picked later");

        // Other tools keep their brushes through Reset Tool.
        let mut a = app();
        pick(&mut a, other(tool));
        scramble(&mut a);
        let kept = live(&a);
        pick(&mut a, tool);
        invoke(&mut a, "tool.reset", json!({}));
        let theirs = a.ui.tool_brushes.iter().find(|(t, _)| *t == other(tool)).map(|(_, b)| tip(b));
        assert_eq!(theirs, Some(kept), "{tool:?}: {:?} keeps its tip", other(tool));

        // First use, after another tool's tip; its own tip is then kept across tool switches.
        let Some(rule) = first_use else { continue };
        let mut a = app();
        pick(&mut a, other(tool));
        scramble(&mut a);
        let previous = live(&a);
        pick(&mut a, tool);
        let first_smoothing = starting(&a, other(tool)).smoothing;
        let want = match rule {
            FirstUse::Fresh => starting(&a, tool),
            FirstUse::SizeOnly => Tip { size: starting(&a, tool).size, smoothing: first_smoothing, ..previous },
            FirstUse::Current => Tip { smoothing: first_smoothing, ..previous },
        };
        assert_eq!(live(&a), want, "{tool:?}: first use ({rule:?})");
        a.session.tools.brush.size = 42.0;
        pick(&mut a, other(tool));
        pick(&mut a, tool);
        assert_eq!(live(&a).size, 42.0, "{tool:?}: its own size is kept across tool switches");
    }
}
