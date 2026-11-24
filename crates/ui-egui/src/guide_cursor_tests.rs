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
    // Two moves: the guide follows the pointer, so one intermediate point is enough to drive the drag and the
    // fourth one only cost another whole-app frame each row (#15941).
    for k in 1..=2 {
        let q = screen(h, from + (to - from) * k as f32 / 2.0, 150.0);
        h.event(egui::Event::PointerMoved(q));
        h.run_steps(1);
    }
    let q = screen(h, to, 150.0);
    h.event(egui::Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers: m });
    h.run_steps(2);
}

/// The vertical guides as they stand.
fn guides(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<f64> {
    h.state().session.active().unwrap().doc.guides.vertical.clone()
}

/// Put the guide back at 200 after a row dragged it, through the ruler's own command, so the next row starts
/// where the first one did without paying for another app.
fn put_guide_back(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.state_mut().run("view.moveGuide", json!({"orientation": "vertical", "index": 0, "position": 200})).unwrap();
    h.run_steps(2);
    assert_eq!(guides(h), vec![200.0], "the guide is back at 200 for the next row");
}

/// The cursor over the guide with ⌘ held (or latched in Window › Modifier Keys, with no key held),
/// and where a ⌘-drag from 200 to 240 leaves it, on the shared harness `h`.
fn command_drag(h: &mut Harness<'static, Box<PhotocraftApp>>, tool: Tool, locked: bool, latched: bool) -> (egui::CursorIcon, Vec<f64>) {
    h.state_mut().ui.tool = tool;
    h.state_mut().ui.extras.lock_guides = locked;
    let held = if latched {
        h.state_mut().ui.shell.latched = craft::keymap::Mods::CMD;
        Modifiers::NONE
    } else {
        h.state_mut().ui.shell.latched = craft::keymap::Mods::NONE;
        Modifiers::COMMAND
    };
    h.run_steps(1);
    let c = hover(h, 200.0, 150.0, held);
    drag(h, 200.0, 240.0, held);
    (c, guides(h))
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
    let check = |h: &mut Harness<'static, Box<PhotocraftApp>>, tool: Tool, locked: bool, latched: bool, takes_guide: bool| {
        let (cursor, moved) = command_drag(h, tool, locked, latched);
        if takes_guide {
            assert_eq!((cursor, moved), (GUIDE, vec![240.0]), "{tool:?} latched={latched}");
            put_guide_back(h);
        } else {
            assert_ne!(cursor, GUIDE, "{tool:?} locked={locked}");
            assert_eq!(moved, vec![200.0], "{tool:?} locked={locked}");
        }
    };
    // One app for the rows whose ⌘ press the Move tool takes and for the rows that ask a locked guide to stay:
    // `harness` builds the whole app and its settle frames, while the selected tool, the ⌘ latch and the locked
    // guides are state the app reads each frame. The split is by what a row expects, not by tool name: a takes-row
    // puts the guide back with the ruler's own command (`view.moveGuide`, what a user does to move it again) and a
    // locked row moves nothing, so neither leaves state for the rows after it (#15941).
    let mut shared = harness(Tool::Brush);
    let mut ran = 0;
    for (tool, locked, latched, takes_guide) in rows {
        if takes_guide || locked {
            check(&mut shared, tool, locked, latched, takes_guide);
            ran += 1;
        }
    }
    // The tools that keep ⌘ for themselves share an app too. The Type tool goes last: its ⌘-drag leaves an open
    // text edit, and `moves_with_command` refuses the guides while one is open, so a later row would be decided by
    // that instead of by its tool. The Crop tool keeps an app of its own for the same reason: its drag leaves a
    // pending crop frame holding the canvas.
    let mut keeps = harness(Tool::Pen);
    for tool in [Tool::Pen, Tool::Hand, Tool::Zoom, Tool::Rectangle, Tool::Slice, Tool::Type] {
        // Cheap guard on the reason this app is ordered: no row so far left an open text edit that would decide
        // this one instead of its tool.
        assert!(keeps.state().ui.text_edit.is_none(), "no text edit open before {tool:?} in the keeps app");
        check(&mut keeps, tool, false, false, false);
        ran += 1;
    }
    let mut crop = harness(Tool::Crop);
    check(&mut crop, Tool::Crop, false, false, false);
    ran += 1;
    assert_eq!(ran, rows.len(), "every row ran once: {} shared, 6 keeps, 1 crop", ran - 7);
}
