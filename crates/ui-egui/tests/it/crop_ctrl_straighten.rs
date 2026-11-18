//! Crop tool: a Straighten line drawn with ⌘ (Ctrl) held turns the image but keeps the frame's
//! size (#10870).
//!
//! Measured in the reference application: an 800 × 500 document, the frame around the whole
//! canvas, a line rising about 10°. ⌘ held commits 800 × 500 with the uncovered corners in the
//! background colour (Fill: Background).
//!
//! Driven as an agent does, through `ui.pointer` (document px), in both crop modes. The app runs
//! in an egui_kittest harness (no GPU needed): the canvas takes each `ui.pointer` call's input on
//! the frames that follow it.

use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use photocraft_ui_egui::crop_ui::{frame, pending};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The background colour, #1050e0: what the turn uncovers is filled with it.
const BLUE: [f32; 3] = [16.0 / 255.0, 80.0 / 255.0, 224.0 / 255.0];

/// An 800 × 500 white document with a red 40 px square centred on (300, 200), the Crop tool
/// picked in Classic Mode or the default mode, and its default frame.
fn app(classic: bool) -> Harness {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"width": 800, "height": 500, "background": "white"})).expect("new");
        app.run("select.rect", json!({"x": 280, "y": 180, "width": 40, "height": 40})).expect("select");
        app.run("edit.fill", json!({"color": "#ff0000"})).expect("fill");
        app.run("select.deselect", json!({})).expect("deselect");
        app.run("tools.setColors", json!({"background": "#1050e0"})).expect("colors");
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.ui.tool_options.crop.shield.classic_mode = classic;
        app
    });
    h.run_steps(4);
    control(&mut h, "ui.set", json!({"tool": "crop"}));
    h
}

/// Runs one control-channel request, which must succeed, then frames so the canvas takes any
/// queued input.
fn control(h: &mut Harness, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    match handle(h.state_mut(), &ctx, &req) {
        Reply::Done(v) => assert_eq!(v["ok"], true, "{method} {params}: {v}"),
        Reply::AfterInput => {}
        _ => panic!("{method} {params}: not done"),
    }
    h.run_steps(6);
}

/// One `ui.pointer` call: `events` with ⌘ held or not.
fn pointer(h: &mut Harness, command: bool, events: Value) {
    control(h, "ui.pointer", json!({"command": command, "events": events}));
}

/// A drag from `a` to `b` through their midpoint, `command` held or not.
fn drag(h: &mut Harness, command: bool, a: [f64; 2], b: [f64; 2]) {
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let events = json!([
        {"kind": "down", "x": a[0], "y": a[1]},
        {"kind": "move", "x": mid[0], "y": mid[1]},
        {"kind": "move", "x": b[0], "y": b[1]},
        {"kind": "up", "x": b[0], "y": b[1]},
    ]);
    pointer(h, command, events);
}

/// The issue's line's ends: rising 10° to the right across the canvas.
fn ends() -> ([f64; 2], [f64; 2]) {
    ([100.0, 300.0], [700.0, 300.0 - 600.0 * 10.0f64.to_radians().tan()])
}

/// The issue's line, `command` held or not.
fn line(h: &mut Harness, command: bool) {
    let (a, b) = ends();
    drag(h, command, a, b);
}

/// The crop frame's rect as `[x0, y0, x1, y1]` (document px).
fn rect(h: &Harness) -> Option<[f64; 4]> {
    frame(h.state()).map(|f| [f.rect.x0, f.rect.y0, f.rect.x1, f.rect.y1])
}

/// The crop frame's turn in degrees (0 with no frame).
fn angle(h: &Harness) -> f64 {
    frame(h.state()).map_or(0.0, |f| f.angle)
}

/// Commits the crop (`image.crop`, as ↵ does) and returns the document's new size.
fn committed(h: &mut Harness) -> (u32, u32) {
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "image.crop", json!({})).expect("crop");
    h.run_steps(6);
    let size = h.state().session.active().expect("document").doc.size;
    (size.width, size.height)
}

/// The flattened document's pixel at (`x`, `y`).
fn pixel(h: &Harness, x: usize, y: usize) -> [f32; 4] {
    let doc = &h.state().session.active().expect("document").doc;
    let flat = photocraft_compose::flatten(doc);
    flat.px[y * doc.size.width as usize + x]
}

fn is_blue(p: [f32; 4]) -> bool {
    BLUE.iter().zip(p).all(|(w, g)| (w - g).abs() < 0.02)
}

fn is_white(p: [f32; 4]) -> bool {
    p[..3].iter().all(|&v| v > 0.98)
}

fn is_red(p: [f32; 4]) -> bool {
    p[0] > 0.98 && p[1] < 0.02 && p[2] < 0.02
}

/// Undo steps the box back before ↵ (the frame upright, the default again) and Redo turns it
/// again; after ↵ the crop is one document step that Undo reverts.
#[test]
fn a_command_held_line_undoes_as_one_box_step_and_commits_as_one_document_step() {
    for classic in [false, true] {
        let mut h = app(classic);
        line(&mut h, true);
        h.state_mut().run("edit.undo", json!({})).expect("undo");
        h.run_steps(6);
        assert_eq!((rect(&h), angle(&h)), (Some([0.0, 0.0, 800.0, 500.0]), 0.0), "classic {classic}: undone");
        assert!(!pending(h.state()), "classic {classic}: the default frame again");
        h.state_mut().run("edit.redo", json!({})).expect("redo");
        h.run_steps(6);
        // The line's ends pass through f32 screen points, so the angle is -10 only to f32 precision (about 1e-6 degrees
        // for ends hundreds of points apart); 1e-4 is the tolerance.
        assert!((angle(&h) + 10.0).abs() < 1e-4 && pending(h.state()), "classic {classic}: redone");
        assert_eq!(rect(&h), Some([0.0, 0.0, 800.0, 500.0]), "classic {classic}: redone");
        let steps = h.state().session.active().expect("document").history.past_len();
        assert_eq!(committed(&mut h), (800, 500), "classic {classic}");
        assert_eq!(h.state().session.active().expect("document").history.past_len(), steps + 1, "classic {classic}");
        assert!(is_blue(pixel(&h, 0, 0)), "classic {classic}: filled");
        h.state_mut().run("edit.undo", json!({})).expect("undo the crop");
        h.run_steps(6);
        assert!(is_white(pixel(&h, 0, 0)) && is_red(pixel(&h, 300, 200)), "classic {classic}: the image as it was");
    }
}
