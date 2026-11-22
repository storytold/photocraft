//! #6288: ⌘ (Ctrl off the Mac) over a guide is the Move tool's guide drag with every tool on
//! which ⌘ is the Move tool, and the pointer shows it (the guide's double arrow), as in
//! Photoshop. Driven through real egui input, so the held key goes through `hold_keys`.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The canvas at 100 % on a 400 × 300 document with `tool` and a vertical guide at x = 200.
pub(crate) fn harness(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 200})).unwrap();
    // The whole app: its frame hook hands the keys and the pointer to the framework's runtime.
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(ctx).with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |_cc| app);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

pub(crate) fn screen(h: &Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32) -> Pos2 {
    crate::canvas::testing::to_screen(h.state(), [f64::from(x), f64::from(y)])
}

pub(crate) fn hover(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32, m: Modifiers) -> egui::CursorIcon {
    h.event(egui::Event::ModifiersChanged(m));
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(2);
    h.output().platform_output.cursor_icon
}

fn drag(h: &mut Harness<'static, Box<PhotocraftApp>>, from: f32, to: f32, m: Modifiers) {
    let p = screen(h, from, 150.0);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: m });
    h.run_steps(1);
    for k in 1..=4 {
        let q = screen(h, from + (to - from) * k as f32 / 4.0, 150.0);
        h.event(egui::Event::PointerMoved(q));
        h.run_steps(1);
    }
    let q = screen(h, to, 150.0);
    h.event(egui::Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers: m });
    h.run_steps(2);
}

/// The cursor over the guide with ⌘ held (or latched in Window › Modifier Keys, with no key held),
/// and where a ⌘-drag from 200 to 240 leaves it.
fn command_drag(tool: Tool, locked: bool, latched: bool) -> (egui::CursorIcon, Vec<f64>) {
    let mut h = harness(tool);
    h.state_mut().ui.extras.lock_guides = locked;
    let held = if latched {
        h.state_mut().ui.shell.latched = craft::keymap::Mods::CMD;
        Modifiers::NONE
    } else {
        Modifiers::COMMAND
    };
    let c = hover(&mut h, 200.0, 150.0, held);
    drag(&mut h, 200.0, 240.0, held);
    (c, h.state().session.active().unwrap().doc.guides.vertical.clone())
}

const GUIDE: egui::CursorIcon = egui::CursorIcon::ResizeHorizontal;

/// (tool, guides locked, ⌘ latched instead of held) → whether ⌘ takes the guide: the guide cursor
/// over it, and the guide after a drag from 200 to 240. ⌘ is the Move tool on the painting, selection,
/// Frame and Artboard tools. The tools that keep ⌘ for themselves (Photoshop: the Pen's Direct
/// Selection, the Hand, Zoom, Crop, type, shapes, slices) neither show the guide cursor nor drag
/// the guide, and a locked guide never moves. A latched ⌘ takes the guide and shows its cursor as a
/// held one does (the press took the guide but the cursor stayed the brush's).
#[test]
fn command_over_a_guide_takes_it_where_command_is_the_move_tool() {
    let takes = |tool, latched| (tool, false, latched, true);
    // One tool per family (`canvas::tools::family`): whether the guides take ⌘ is decided by family
    // (`rulers::moves_with_command`, `tools::takes_command_stand_in`), and the painting family refuses a ⌘ press the
    // same way for all its tools (`PaintTool::takes`). Within it, one tool per kind of pointer (`cursor_policy`): the
    // Brush's tip with its centre mark, the Clone Stamp's and Quick Selection's tips without one, the Paint Bucket's
    // click-tool pointer. The Eraser, Dodge, Spot Healing Brush and Smudge are more Brush-like tips.
    let rows = [
        takes(Tool::Brush, false),
        takes(Tool::Gradient, false),
        takes(Tool::PaintBucket, false),
        takes(Tool::CloneStamp, false),
        takes(Tool::Eyedropper, false),
        takes(Tool::QuickSelection, false),
        takes(Tool::RectMarquee, false),
        takes(Tool::Lasso, false),
        // The Frame and Artboard tools took a ⌘ press as their own (a new frame or board) although ⌘
        // is the Move tool on them, so the guide cursor promised a drag that never moved the guide.
        takes(Tool::Frame, false),
        takes(Tool::Artboard, false),
        takes(Tool::Brush, true),
        takes(Tool::RectMarquee, true),
        (Tool::Brush, true, false, false),
        (Tool::Frame, true, false, false),
        (Tool::Pen, false, false, false),
        (Tool::Hand, false, false, false),
        (Tool::Zoom, false, false, false),
        (Tool::Crop, false, false, false),
        (Tool::Type, false, false, false),
        (Tool::Rectangle, false, false, false),
        (Tool::Slice, false, false, false),
    ];
    for (tool, locked, latched, takes_guide) in rows {
        let (cursor, guides) = command_drag(tool, locked, latched);
        if takes_guide {
            assert_eq!((cursor, guides), (GUIDE, vec![240.0]), "{tool:?} latched={latched}");
        } else {
            assert_ne!(cursor, GUIDE, "{tool:?} locked={locked}");
            assert_eq!(guides, vec![200.0], "{tool:?} locked={locked}");
        }
    }
}
