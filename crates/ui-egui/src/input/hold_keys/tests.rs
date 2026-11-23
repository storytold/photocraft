//! Held keys through the whole app: Space repositions a selection being drawn, ⌘ gives the Move
//! tool where the tool does not take ⌘, `~` erases with the Brush and Pencil, a held key keeps a
//! pending crop, and Space right-click opens the Hand's view menu. Keys and buttons go in as
//! window events, so they reach the runtime and come back as fired commands.

use craft::behavior::cursor::{Cursor, PaintedCursor};
use craft::behavior::kurbo::Point;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_geom::Rect;
use serde_json::json;

use super::*;
use crate::canvas::testing;
use crate::canvas_tool_menu::MenuKind;

/// A 400 × 300 document in the whole app, shown at 100 %, with `tool` selected.
fn harness(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.ui.tool = tool;
        app
    });
    h.run_steps(8);
    tap(&mut h, Key::Num0, Modifiers::COMMAND | Modifiers::ALT);
    h.run_steps(2);
    h
}

/// The screen point of the document point (`x`, `y`) in the canvas the app shows.
fn screen(h: &Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32) -> Pos2 {
    testing::to_screen(h.state(), [f64::from(x), f64::from(y)])
}

/// The document point at the centre of the shown canvas.
fn center(h: &Harness<'static, Box<PhotocraftApp>>) -> Point {
    testing::view(h.state()).expect("the canvas shows a view").center
}

/// The shown canvas's zoom.
fn zoom(h: &Harness<'static, Box<PhotocraftApp>>) -> f64 {
    testing::view(h.state()).expect("the canvas shows a view").zoom
}

/// The platform's modifiers for `Cmd` (⌘ on the Mac, Ctrl elsewhere), as egui-winit reports them.
fn platform(m: Modifiers) -> Modifiers {
    let mut out = Modifiers { alt: m.alt, shift: m.shift, ctrl: m.ctrl, ..Default::default() };
    if m.command {
        out.command = true;
        if cfg!(target_os = "macos") {
            out.mac_cmd = true;
        } else {
            out.ctrl = true;
        }
    }
    out
}

fn key(h: &mut Harness<'static, Box<PhotocraftApp>>, k: Key, pressed: bool, m: Modifiers) {
    let m = platform(m);
    h.event(Event::ModifiersChanged(m));
    h.event(Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: m });
    // The runtime takes the key in the first frame and the app runs the command it fired in the next.
    h.run_steps(2);
}

/// A quick press and release in one frame (a tool letter held across frames springs, #2672).
fn tap(h: &mut Harness<'static, Box<PhotocraftApp>>, k: Key, m: Modifiers) {
    let m = platform(m);
    h.input_mut().events.extend([
        Event::ModifiersChanged(m),
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m },
        Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: m },
    ]);
    h.run_steps(1);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

fn button(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, down: bool, m: Modifiers) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: platform(m) });
    h.run_steps(1);
}

fn move_to(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(Event::PointerMoved(p));
    h.run_steps(2);
}

fn press_at(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(Event::PointerMoved(p));
    h.run_steps(1);
    button(h, p, true, Modifiers::NONE);
    move_to(h, x + 8.0, y + 6.0);
}

fn release(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32, m: Modifiers) {
    let p = screen(h, x, y);
    button(h, p, false, m);
}

fn selection(h: &Harness<'static, Box<PhotocraftApp>>) -> Rect {
    h.state().session.active().unwrap().doc.selection.as_ref().map_or(Rect::EMPTY, |s| s.content_bounds())
}

fn click(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32, m: Modifiers) {
    let p = screen(h, x, y);
    h.event(Event::PointerMoved(p));
    h.run_steps(1);
    button(h, p, true, m);
    button(h, p, false, m);
    h.run_steps(1);
}

/// Changes only the held modifiers, as pressing or releasing ⌥ / ⌘ does with a key already down.
fn hold_mods(h: &mut Harness<'static, Box<PhotocraftApp>>, m: Modifiers) {
    h.event(Event::ModifiersChanged(platform(m)));
    h.run_steps(1);
}

/// One drag with Space repositioning it: the points before Space goes down, those while it is
/// held, and those after it comes up.
struct Reposition {
    tool: Tool,
    start: (f32, f32),
    before: &'static [(f32, f32)],
    held: &'static [(f32, f32)],
    after: &'static [(f32, f32)],
    /// Space comes up before the button does (sizing resumes) instead of after it.
    space_up_first: bool,
    /// The selection's bounds and how far each edge may be off.
    expect: Rect,
    slack: u32,
}

/// With Space held mid-drag the outline moves and the view does not pan; releasing Space goes
/// back to sizing from the moved corner. The runtime's momentary Space has to reach the selection
/// tool mid-gesture instead of the Hand.
#[test]
fn space_repositions_the_drag_being_drawn() {
    let cases = [
        // Released with Space still down: the 100 × 60 rectangle, moved by (80, 60).
        Reposition {
            tool: Tool::RectMarquee,
            start: (50.0, 50.0),
            before: &[(150.0, 110.0)],
            held: &[(200.0, 150.0), (230.0, 170.0)],
            after: &[],
            space_up_first: false,
            expect: Rect::new(130, 110, 230, 170),
            slack: 0,
        },
        // Space up mid-drag: sizing continues from the moved corner.
        Reposition {
            tool: Tool::EllipseMarquee,
            start: (50.0, 50.0),
            before: &[(150.0, 110.0)],
            held: &[(170.0, 120.0)],
            after: &[(200.0, 140.0)],
            space_up_first: true,
            expect: Rect::new(70, 60, 200, 140),
            slack: 1,
        },
        Reposition {
            tool: Tool::Lasso,
            start: (50.0, 50.0),
            before: &[(120.0, 50.0), (120.0, 100.0)],
            held: &[(170.0, 130.0)],
            after: &[],
            space_up_first: false,
            expect: Rect::new(100, 80, 170, 130),
            slack: 1,
        },
    ];
    // One app for the three marquee and lasso cases: the tool is state, and each case starts from no selection.
    let mut h = harness(Tool::RectMarquee);
    for c in cases {
        h.state_mut().run("select.deselect", json!({})).ok();
        assert_eq!(selection(&h), Rect::EMPTY, "{:?}: the case starts with no selection", c.tool);
        h.state_mut().ui.tool = c.tool;
        h.run_steps(1);
        press_at(&mut h, c.start.0, c.start.1);
        for &(x, y) in c.before {
            move_to(&mut h, x, y);
        }
        let panned_from = center(&h);
        key(&mut h, Key::Space, true, Modifiers::NONE);
        for &(x, y) in c.held {
            move_to(&mut h, x, y);
        }
        if c.space_up_first {
            key(&mut h, Key::Space, false, Modifiers::NONE);
        }
        for &(x, y) in c.after {
            move_to(&mut h, x, y);
        }
        let end = c.after.last().or(c.held.last()).copied().unwrap();
        release(&mut h, end.0, end.1, Modifiers::NONE);
        if !c.space_up_first {
            key(&mut h, Key::Space, false, Modifiers::NONE);
        }
        h.run_steps(2);
        let r = selection(&h);
        let near = |a: i32, b: i32| a.abs_diff(b) <= c.slack;
        assert!(near(r.x0, c.expect.x0) && near(r.y0, c.expect.y0) && near(r.x1, c.expect.x1) && near(r.y1, c.expect.y1), "{:?}: {r:?}", c.tool);
        assert_eq!(center(&h), panned_from, "{:?}: Space didn't pan", c.tool);
        assert_eq!(h.state().ui.tool, c.tool);
    }
}

/// The open paths the last frame painted, as their screen points.
fn painted_paths(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<Vec<Pos2>> {
    fn walk(s: &egui::Shape, out: &mut Vec<Vec<Pos2>>) {
        match s {
            egui::Shape::Path(p) if !p.closed => out.push(p.points.clone()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
}

#[test]
fn the_pen_path_being_drawn_stays_visible_while_space_pans() {
    // #6592: the path vanished for as long as the Hand dragged.
    let mut h = harness(Tool::Pen);
    click(&mut h, 100.0, 100.0, Modifiers::NONE);
    click(&mut h, 200.0, 100.0, Modifiers::NONE);
    assert_eq!(h.state().ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(2));
    key(&mut h, Key::Space, true, Modifiers::NONE);
    let before = center(&h);
    press_at(&mut h, 150.0, 200.0);
    move_to(&mut h, 190.0, 230.0);
    assert_ne!(center(&h), before, "the view is panning");
    // Mid-drag: the segment is painted where its anchors now are on screen.
    let (a, b) = (screen(&h, 100.0, 100.0), screen(&h, 200.0, 100.0));
    let near = |p: &Pos2, q: Pos2| p.distance(q) < 1.0;
    assert!(
        painted_paths(&h).iter().any(|pts| pts.first().is_some_and(|p| near(p, a)) && pts.last().is_some_and(|p| near(p, b))),
        "the Pen path from {a:?} to {b:?} is drawn while panning"
    );
    // The host paints the Hand's cursor, closed while the button is down (#2196).
    let cursor = h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone());
    assert_eq!(cursor, Some(Cursor::Painted(PaintedCursor::Hand)), "the Hand's cursor while panning (#2196)");
    release(&mut h, 190.0, 230.0, Modifiers::NONE);
    key(&mut h, Key::Space, false, Modifiers::NONE);
    h.run_steps(2);
    let st = h.state();
    assert_eq!(st.ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(2), "the path is still being drawn");
    assert_eq!(st.ui.tool, Tool::Pen);
}

fn rgba(h: &Harness<'static, Box<PhotocraftApp>>, x: i32, y: i32) -> [f32; 4] {
    h.state().session.active().unwrap().doc.layers.last().unwrap().surface().unwrap().rgba(x, y)
}

#[test]
fn fill_key_on_a_locked_layer_reports_instead_of_crashing() {
    let mut h = harness(Tool::Brush);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.state_mut().run("layer.lockLayers", json!({"pixels": true})).unwrap();
    tap(&mut h, Key::Backspace, Modifiers::ALT);
    let st = h.state();
    assert!(st.ui.status_error && st.ui.status.contains("locked"), "{}", st.ui.status);
    assert_eq!(rgba(&h, 5, 5)[3], 0.0);
}

/// `ui.pointer` queues its input on the tool host; the next passes deliver it, so each call is
/// followed by frames before the selection is read.
#[test]
fn agents_reposition_a_marquee_with_space() {
    use crate::control::{ControlRequest, Reply, handle};
    fn call(app: &mut PhotocraftApp, p: serde_json::Value) {
        let ctx = testing::ctx(app);
        let (req, _rx) = ControlRequest::new("ui.pointer", p);
        assert!(matches!(handle(app, &ctx, &req), Reply::Done(_)));
        for _ in 0..4 {
            testing::frame(app);
        }
    }
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    testing::frame(&mut app);
    call(&mut app, json!({"tool": "rectMarquee", "events": [{"kind": "down", "x": 20, "y": 20}, {"kind": "move", "x": 120, "y": 80}]}));
    call(&mut app, json!({"space": true, "events": [{"kind": "move", "x": 170, "y": 110}]}));
    call(&mut app, json!({"space": true, "events": [{"kind": "up", "x": 170, "y": 110}]}));
    let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
    assert_eq!(sel, Rect::new(70, 50, 170, 110));
}

/// A red square on a new layer for the ⌘ (Move) tests; Auto-Select, snapping and smart guides
/// off so the drag offsets are exact.
fn with_layer(h: &mut Harness<'static, Box<PhotocraftApp>>) -> photocraft_doc::LayerId {
    let app = h.state_mut();
    app.run("layer.new.layer", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(100, 100, 160, 160), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    h.run_steps(2);
    id
}

fn bounds(h: &Harness<'static, Box<PhotocraftApp>>, id: photocraft_doc::LayerId) -> Rect {
    h.state().session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

/// A drag with `m` held throughout (the modifiers go down before the press, as on a keyboard).
fn drag_with(h: &mut Harness<'static, Box<PhotocraftApp>>, m: Modifiers, x: f32, y: f32, x2: f32, y2: f32) {
    drag_raw(h, platform(m), x, y, x2, y2);
}

/// [`drag_with`] with the modifiers exactly as given, for one platform's layout on any host.
fn drag_raw(h: &mut Harness<'static, Box<PhotocraftApp>>, m: Modifiers, x: f32, y: f32, x2: f32, y2: f32) {
    h.event(Event::ModifiersChanged(m));
    h.run_steps(1);
    let p = screen(h, x, y);
    h.event(Event::PointerMoved(p));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: m });
    h.run_steps(1);
    move_to(h, x + 8.0, y + 6.0);
    move_to(h, x2, y2);
    let q = screen(h, x2, y2);
    h.event(Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers: m });
    h.run_steps(1);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

/// ⌘⌥ as a Mac sends it, and Ctrl+Alt as Windows and Linux send it (`command` is Ctrl there).
const MAC_CMD_ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: false, mac_cmd: true, command: true };
const CTRL_ALT: Modifiers = Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: true };

/// A ⌘-drag moves the layer through the Move stand-in with the tools that do not take ⌘ (painting,
/// retouching, eraser, gradient, eyedropper, and the Type tools with no type box open, #9994) and
/// with the selection tools (⌘ outside the selection moves the layer, #896): one "Move" step,
/// nothing painted, no text started, the tool still selected. The tools that keep ⌘ for themselves
/// leave the layer alone.
#[test]
fn cmd_gives_the_move_tool_only_where_the_tool_does_not_take_cmd() {
    // The Eraser is left out: it is in the Brush's family (`canvas::tools::family`), which refuses a ⌘ press the same
    // way for every tool (`PaintTool::takes`); the Elliptical Marquee is the Rectangular one's marquee pass with
    // another shape (`selection.rs`, `MarqueeKind`).
    // One app per group, with the tool switched between cases and a fresh layer for each: starting
    // the whole app dominates the cost of a case.
    let moves = [Tool::Brush, Tool::Gradient, Tool::CloneStamp, Tool::Eyedropper, Tool::Move, Tool::RectMarquee, Tool::Lasso, Tool::Type, Tool::VerticalType];
    let mut h = harness(moves[0]);
    for tool in moves {
        h.state_mut().ui.tool = tool;
        h.run_steps(1);
        let id = with_layer(&mut h);
        let steps = h.state().session.active().unwrap().history.past_len();
        drag_with(&mut h, Modifiers::COMMAND, 130.0, 130.0, 170.0, 150.0);
        let st = h.state().session.active().unwrap();
        assert_eq!(bounds(&h, id), Rect::new(140, 120, 200, 180), "{tool:?}: the layer moved by (40, 20)");
        assert_eq!(st.history.undo_label(), Some("Move"), "{tool:?}");
        assert_eq!(st.history.past_len(), steps + 1, "{tool:?}: one step, nothing painted");
        assert_eq!(h.state().ui.tool, tool, "{tool:?}: the tool is back");
        assert!(h.state().ui.text_edit.is_none(), "{tool:?}: no text was started");
    }
    // The Crop and Rotate View tools go last, in an app of their own: they leave a pending crop and
    // a turned view behind.
    // One tool per decision the refusal comes from: the Hand, Zoom and Rotate-View families and the vector family's
    // shared line (the Pen stands for the shapes and both path tools). Every tool's own decision is asserted in
    // `the_command_stand_in_rule_is_the_table`, without a drag.
    let keeps = [vec![Tool::Hand, Tool::Zoom, Tool::Pen], vec![Tool::Crop, Tool::RotateView]];
    for group in keeps {
        let mut h = harness(group[0]);
        for tool in group {
            h.state_mut().ui.tool = tool;
            h.run_steps(1);
            keeps_the_layer(&mut h, tool);
        }
    }
}

/// The ⌘-as-Move rule read as data: `takes_command_stand_in` and the selection family decide which tool hands a ⌘
/// press to the Move tool, for every tool in the app. The drags above press one tool per decision; this pins each
/// tool by name, so a tool that moves into another family or starts refusing the stand-in fails here.
#[test]
fn the_command_stand_in_rule_is_the_table() {
    use crate::canvas::tools::Family;
    let hands_to_move = |t: Tool| crate::canvas::tools::takes_command_stand_in(t) || crate::canvas::tools::family(t) == Family::Selection;
    let got: Vec<Tool> = Tool::ALL.into_iter().filter(|t| hands_to_move(*t)).collect();
    let want: Vec<Tool> = vec![
        Tool::Artboard,
        Tool::RectMarquee,
        Tool::EllipseMarquee,
        Tool::SingleRowMarquee,
        Tool::SingleColumnMarquee,
        Tool::Lasso,
        Tool::PolygonLasso,
        Tool::MagneticLasso,
        Tool::MagicWand,
        Tool::Eyedropper,
        Tool::ColorSampler,
        Tool::Ruler,
        Tool::Note,
        Tool::Count,
        Tool::Brush,
        Tool::Pencil,
        Tool::ColorReplacement,
        Tool::MixerBrush,
        Tool::Eraser,
        Tool::BackgroundEraser,
        Tool::MagicEraser,
        Tool::Gradient,
        Tool::PaintBucket,
        Tool::Type,
        Tool::VerticalType,
        Tool::HorizontalTypeMask,
        Tool::VerticalTypeMask,
        Tool::SpotHealing,
        Tool::Remove,
        Tool::Healing,
        Tool::Patch,
        Tool::ContentAwareMove,
        Tool::RedEye,
        Tool::CloneStamp,
        Tool::PatternStamp,
        Tool::HistoryBrush,
        Tool::ArtHistoryBrush,
        Tool::Blur,
        Tool::Sharpen,
        Tool::Smudge,
        Tool::AdjustmentBrush,
        Tool::Dodge,
        Tool::Burn,
        Tool::Sponge,
        Tool::QuickSelection,
        Tool::ObjectSelection,
        Tool::Frame,
    ];
    assert_eq!(got, want, "which tools hand a ⌘ press to the Move tool");
}

/// A ⌘-drag with `tool`, one that keeps ⌘ for itself, leaves a fresh layer where it is.
fn keeps_the_layer(h: &mut Harness<'static, Box<PhotocraftApp>>, tool: Tool) {
    let id = with_layer(h);
    drag_with(h, Modifiers::COMMAND, 130.0, 130.0, 170.0, 150.0);
    assert_eq!(bounds(h, id), Rect::new(100, 100, 160, 160), "{tool:?}: the layer stays");
    assert_ne!(h.state().session.active().unwrap().history.undo_label(), Some("Move"), "{tool:?}");
    assert_eq!(h.state().ui.tool, tool);
}

/// ⌘⌥-drag duplicates the layer first and moves the copy, as the Move tool's ⌥-drag does. Off
/// the Mac the same keys are Ctrl+Alt, which the Brush keeps for its on-canvas resize
/// (`brush_resize`, Alt+right-drag there too), so Ctrl+Alt duplicates with the tools that have no
/// brush to resize, such as the Gradient.
#[test]
fn cmd_alt_drag_duplicates_the_layer_then_moves_the_copy() {
    // One app, one fresh layer per case (the `with_layer` each case already calls).
    let mut h = harness(Tool::Brush);
    for (tool, mods) in [(Tool::Brush, MAC_CMD_ALT), (Tool::Gradient, MAC_CMD_ALT), (Tool::Gradient, CTRL_ALT)] {
        h.state_mut().ui.tool = tool;
        h.run_steps(1);
        let orig = with_layer(&mut h);
        let n0 = h.state().session.active().unwrap().doc.layers.len();
        let h0 = h.state().session.active().unwrap().history.past_len();
        drag_raw(&mut h, mods, 130.0, 130.0, 170.0, 150.0);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), n0 + 1, "one copy ({tool:?}, {mods:?})");
        let copy = st.active_layer.unwrap();
        assert_ne!(copy, orig);
        assert_eq!(bounds(&h, orig), Rect::new(100, 100, 160, 160), "the original stays");
        assert_eq!(bounds(&h, copy), Rect::new(140, 120, 200, 180), "the copy moved");
        assert_eq!(st.history.past_len(), h0 + 1, "one undo step");
        // A drag that copies a layer is one "Move" step, as in Photoshop (#11383).
        assert_eq!(st.history.undo_label(), Some(tl!("Move")));
        assert_eq!(h.state().ui.tool, tool);
    }
}

/// With a selection, ⌘-drag with the Brush moves the selected pixels (a floating piece), as the
/// Move tool does, not the whole layer.
#[test]
fn cmd_drag_with_the_brush_inside_the_selection_floats_the_selected_pixels() {
    let mut h = harness(Tool::Brush);
    let id = with_layer(&mut h);
    h.state_mut().run("select.rect", json!({"x": 110, "y": 110, "width": 40, "height": 40})).unwrap();
    h.run_steps(1);
    let n0 = h.state().session.active().unwrap().doc.layers.len();
    drag_with(&mut h, Modifiers::COMMAND, 130.0, 130.0, 170.0, 150.0);
    let st = h.state().session.active().unwrap();
    let f = photocraft_engine::float_cmds::floating(st).expect("a floating piece");
    assert_eq!((f.layer, f.offset), (id, (40, 20)));
    assert_eq!(st.doc.layers.len(), n0, "no layer was added");
    assert_eq!(bounds(&h, id), Rect::new(100, 100, 160, 160), "the layer itself stayed");
}

/// ⌘-click with the Brush picks the layer under the pointer, as a ⌘-click with the Move tool does
/// (Auto-Select off).
#[test]
fn cmd_click_with_the_brush_picks_the_layer_under_the_pointer() {
    let mut h = harness(Tool::Brush);
    let red = with_layer(&mut h);
    let bg = h.state().session.active().unwrap().doc.layers.first().unwrap().id;
    h.state_mut().run("layer.select", json!({"layer": bg.0})).unwrap();
    h.run_steps(1);
    hold_mods(&mut h, Modifiers::COMMAND);
    click(&mut h, 130.0, 130.0, Modifiers::COMMAND);
    hold_mods(&mut h, Modifiers::NONE);
    h.run_steps(2);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.active_layer, Some(red));
    assert_eq!(rgba(&h, 130, 130), [1.0, 0.0, 0.0, 1.0], "nothing painted");
}

/// The text layer's translation.
fn text_offset(h: &Harness<'static, Box<PhotocraftApp>>, id: photocraft_doc::LayerId) -> (f64, f64) {
    match &h.state().session.active().unwrap().doc.layer(id).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => (t.transform.m[4], t.transform.m[5]),
        _ => panic!("not a text layer"),
    }
}

/// The layer count of the active document.
fn layer_count(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().doc.layers.len()
}

/// Whether ⌘ is the Move tool as the app stands: the Move stand-in takes a ⌘ press and a ⌘ press on a guide takes it.
fn cmd_is_move(h: &Harness<'static, Box<PhotocraftApp>>) -> bool {
    let app = h.state();
    let stand_in = crate::canvas::tools::command_stand_in_now(app);
    assert_eq!(crate::canvas::rulers::moves_with_command(app, app.ui.tool), stand_in, "the guides follow the stand-in");
    stand_in
}

/// #9994: with a Type tool, ⌘ is the Move tool once the text is committed: ⌘↵ ends the edit and ⌘
/// still held then nudges the text layer 1 px with the arrows, 10 px with ⇧, and ⌥ duplicates it
/// first, from the arrows or a drag. While the text is edited ⌘ stays the type box's.
#[test]
fn cmd_after_committing_type_moves_and_duplicates_the_text_layer() {
    for tool in [Tool::Type, Tool::VerticalType] {
        let mut h = harness(tool);
        h.state_mut().ui.tool_options.move_auto_select = false;
        h.state_mut().ui.extras.snap = false;
        h.state_mut().ui.view.show.smart_guides = false;
        let id = photocraft_doc::LayerId(
            h.state_mut().run("type.create", json!({"text": "HO", "size": 60, "x": 100, "y": 150})).unwrap()["layer"].as_u64().unwrap(),
        );
        h.run_steps(2);
        click(&mut h, 115.0, 130.0, Modifiers::NONE);
        assert!(h.state().ui.text_edit.is_some(), "{tool:?}: editing");
        hold_mods(&mut h, Modifiers::COMMAND);
        assert!(!cmd_is_move(&h), "{tool:?}: ⌘ is the type box's while editing");
        key(&mut h, Key::Enter, true, Modifiers::COMMAND);
        key(&mut h, Key::Enter, false, Modifiers::COMMAND);
        assert!(h.state().ui.text_edit.is_none(), "{tool:?}: committed");
        assert_eq!(h.state().ui.tool, tool);
        assert!(cmd_is_move(&h), "{tool:?}: committed, ⌘ is the Move tool");
        let (x0, y0) = text_offset(&h, id);
        key(&mut h, Key::ArrowRight, true, Modifiers::COMMAND);
        key(&mut h, Key::ArrowRight, false, Modifiers::COMMAND);
        assert_eq!(text_offset(&h, id), (x0 + 1.0, y0), "{tool:?}: ⌘→ nudges 1 px");
        key(&mut h, Key::ArrowDown, true, Modifiers::COMMAND | Modifiers::SHIFT);
        key(&mut h, Key::ArrowDown, false, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_eq!(text_offset(&h, id), (x0 + 1.0, y0 + 10.0), "{tool:?}: ⌘⇧↓ nudges 10 px");
        let n0 = layer_count(&h);
        key(&mut h, Key::ArrowLeft, true, Modifiers::COMMAND | Modifiers::ALT);
        key(&mut h, Key::ArrowLeft, false, Modifiers::COMMAND | Modifiers::ALT);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), n0 + 1, "{tool:?}: ⌘⌥← duplicates");
        let copy = st.active_layer.unwrap();
        assert_ne!(copy, id);
        assert_eq!(text_offset(&h, id), (x0 + 1.0, y0 + 10.0), "{tool:?}: the original stays");
        assert_eq!(text_offset(&h, copy), (x0, y0 + 10.0), "{tool:?}: the copy moved");
        assert!(h.state().ui.text_edit.is_none(), "{tool:?}: no new text was started");
        // ⌘⌥-drag on the copy: another copy, moved by the drag.
        let (cx, cy) = text_offset(&h, copy);
        let n1 = layer_count(&h);
        drag_raw(&mut h, if cfg!(target_os = "macos") { MAC_CMD_ALT } else { CTRL_ALT }, cx as f32 + 10.0, cy as f32 - 20.0, cx as f32 + 50.0, cy as f32);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), n1 + 1, "{tool:?}: ⌘⌥-drag duplicates");
        let copy2 = st.active_layer.unwrap();
        assert_eq!(text_offset(&h, copy), (cx, cy), "{tool:?}: the dragged-from copy stays");
        assert_eq!(text_offset(&h, copy2), (cx + 40.0, cy + 20.0), "{tool:?}: the new copy moved");
        assert!(h.state().ui.text_edit.is_none(), "{tool:?}");
        assert_eq!(h.state().ui.tool, tool, "{tool:?}: the tool stays");
    }
}

/// An open type box counts as editing whatever is in it, as in Photoshop: ⌘ held is not the Move
/// tool, ⌘-arrows and ⌘⌥-arrows leave the text layer where it is (no nudge, no copy), a ⌘ press
/// on a guide does not take it, and the box stays open on `layer`.
fn assert_cmd_is_the_type_boxs(h: &mut Harness<'static, Box<PhotocraftApp>>, tool: Tool, layer: photocraft_doc::LayerId, why: &str) {
    assert_eq!(h.state().ui.text_edit.as_ref().map(|e| e.layer), Some(layer.0), "{tool:?} {why}: the box is open");
    let (x0, y0) = text_offset(h, layer);
    let n0 = layer_count(h);
    hold_mods(h, Modifiers::COMMAND);
    assert!(!cmd_is_move(h), "{tool:?} {why}: ⌘ is not the Move tool, and takes no guide");
    for m in [Modifiers::COMMAND, Modifiers::COMMAND | Modifiers::SHIFT, Modifiers::COMMAND | Modifiers::ALT] {
        key(h, Key::ArrowRight, true, m);
        key(h, Key::ArrowRight, false, m);
        assert_eq!(text_offset(h, layer), (x0, y0), "{tool:?} {why}: {m:?}-→ does not nudge the layer");
        assert_eq!(layer_count(h), n0, "{tool:?} {why}: {m:?}-→ does not duplicate");
    }
    assert_eq!(h.state().ui.text_edit.as_ref().map(|e| e.layer), Some(layer.0), "{tool:?} {why}: still editing");
    hold_mods(h, Modifiers::NONE);
}

/// The two in-between cases for #9994's ⌘. (1) A new, empty type box (a click with the Type tool,
/// no placeholder text) is being edited: ⌘ keeps its editing meaning. Committing it with ⌘↵
/// removes the empty layer, and only then is ⌘ the Move tool.
#[test]
fn cmd_in_an_open_empty_type_box_stays_the_type_boxs() {
    for tool in [Tool::Type, Tool::VerticalType] {
        let mut h = harness(tool);
        h.state_mut().session.edit_prefs(|p| p.type_.fill_new_type_layers_with_placeholder = false);
        h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 300})).unwrap();
        let n0 = layer_count(&h);
        click(&mut h, 120.0, 150.0, Modifiers::NONE);
        let edit = h.state().ui.text_edit.clone().expect("a click opens a type box");
        assert!(edit.created, "{tool:?}: a new layer");
        let id = photocraft_doc::LayerId(edit.layer);
        match &h.state().session.active().unwrap().doc.layer(id).unwrap().content {
            photocraft_doc::LayerContent::Text(t) => assert!(t.text.is_empty(), "{tool:?}: the box is empty"),
            _ => panic!("not a text layer"),
        }
        assert_cmd_is_the_type_boxs(&mut h, tool, id, "empty box");
        key(&mut h, Key::Enter, true, Modifiers::COMMAND);
        key(&mut h, Key::Enter, false, Modifiers::COMMAND);
        assert!(h.state().ui.text_edit.is_none(), "{tool:?}: committed");
        assert_eq!(layer_count(&h), n0, "{tool:?}: the empty layer is gone");
        assert!(cmd_is_move(&h), "{tool:?}: committed, ⌘ is the Move tool");
        hold_mods(&mut h, Modifiers::NONE);
    }
}

/// (2) Clicking into existing text, or a new box still showing its selected placeholder, without
/// typing anything is editing too: ⌘ stays the type box's until the text is committed.
#[test]
fn cmd_after_clicking_into_type_without_typing_stays_the_type_boxs() {
    for tool in [Tool::Type, Tool::VerticalType] {
        // Existing text, clicked into.
        let mut h = harness(tool);
        h.state_mut().ui.tool_options.move_auto_select = false;
        h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 300})).unwrap();
        let id = photocraft_doc::LayerId(
            h.state_mut().run("type.create", json!({"text": "HO", "size": 60, "x": 100, "y": 150})).unwrap()["layer"].as_u64().unwrap(),
        );
        h.run_steps(2);
        let before = h.state().session.active().unwrap().doc.layer(id).unwrap().content.clone();
        click(&mut h, 115.0, 130.0, Modifiers::NONE);
        assert!(!h.state().ui.text_edit.as_ref().unwrap().created, "{tool:?}: editing the existing layer");
        assert_cmd_is_the_type_boxs(&mut h, tool, id, "clicked into");
        let after = &h.state().session.active().unwrap().doc.layer(id).unwrap().content;
        let text = |c: &photocraft_doc::LayerContent| match c {
            photocraft_doc::LayerContent::Text(t) => t.text.clone(),
            _ => panic!("not a text layer"),
        };
        assert_eq!(text(after), text(&before), "{tool:?}: nothing typed");
        key(&mut h, Key::Enter, true, Modifiers::COMMAND);
        key(&mut h, Key::Enter, false, Modifiers::COMMAND);
        assert!(h.state().ui.text_edit.is_none(), "{tool:?}: committed");
        assert!(cmd_is_move(&h), "{tool:?}: committed, ⌘ is the Move tool");
        hold_mods(&mut h, Modifiers::NONE);

        // A new box with its placeholder selected, nothing typed over it.
        let mut h = harness(tool);
        h.state_mut().session.edit_prefs(|p| p.type_.fill_new_type_layers_with_placeholder = true);
        h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": 300})).unwrap();
        click(&mut h, 120.0, 150.0, Modifiers::NONE);
        let id = photocraft_doc::LayerId(h.state().ui.text_edit.as_ref().unwrap().layer);
        assert_cmd_is_the_type_boxs(&mut h, tool, id, "placeholder");
    }
}

/// Free Transform keeps ⌘ (its distort handles) while the Hand is selected: a ⌘-click is not the
/// Hand's Zoom In. The app tells the Hand's modifier zoom that the Hand is not selected while a
/// transform is open.
#[test]
fn cmd_with_the_hand_tool_is_not_a_zoom_during_free_transform() {
    let mut h = harness(Tool::Hand);
    with_layer(&mut h);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.state().ui.transform.is_some());
    let z0 = zoom(&h);
    hold_mods(&mut h, Modifiers::COMMAND);
    click(&mut h, 200.0, 150.0, Modifiers::COMMAND);
    hold_mods(&mut h, Modifiers::NONE);
    assert_eq!(zoom(&h), z0, "no zoom");
}

/// The params of the last brush stroke (`paint.stroke`, or the Pencil's `paint.pencil`).
fn last_stroke(app: &PhotocraftApp) -> serde_json::Value {
    let journal = &app.session.journal;
    journal.iter().rev().find(|(id, _)| id == "paint.stroke" || id == "paint.pencil").map(|(_, p)| p.clone()).unwrap_or_default()
}

/// The `erase` flag of the last brush stroke.
fn last_stroke_erases(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<bool> {
    last_stroke(h.state())["erase"].as_bool()
}

/// `~` held with the Brush or the Pencil erases with that tool's own brush (#3439): Photoshop's
/// Clear mode for that stroke, so History names the tool. The tool stays, and the next stroke
/// without the key paints again. Other tools have no erasing stroke and ignore the key. The flag
/// holds across frames (a second pass in a frame does not clear it) and goes with the release.
#[test]
fn tilde_erases_only_with_the_brush_and_pencil() {
    // One app, a fresh layer per tool (`with_layer`), so the strokes of one tool never sit under the next one's.
    let mut h = harness(Tool::Brush);
    for (tool, label, erases) in [(Tool::Brush, "Brush Tool", true), (Tool::Pencil, "Pencil", true), (Tool::CloneStamp, "", false)] {
        h.state_mut().ui.tool = tool;
        h.run_steps(1);
        with_layer(&mut h);
        key(&mut h, Key::Backtick, true, Modifiers::NONE);
        h.run_steps(3);
        assert_eq!(h.state().held_erase, erases, "{tool:?}: the held key reaches `erase_key`");
        assert_eq!(crate::paint_mouse::clears(h.state(), tool), erases, "{tool:?}: the mode shows Clear");
        if erases {
            drag_with(&mut h, Modifiers::NONE, 110.0, 130.0, 150.0, 130.0);
            assert_eq!(last_stroke_erases(&h), Some(true), "{tool:?}");
            // `drag_with` bends through `(118, 136)` on the way from `(110, 130)` to `(150, 130)`,
            // so the Pencil's 1 px tip (#12200) no longer reaches `(130, 130)`; check `(110, 130)`
            // at the stroke start instead.
            assert_eq!(rgba(&h, 110, 130)[3], 0.0, "{tool:?}: erased to transparency");
            // The whole drag erases, not only its first dab: a 1 px Pencil line along the 40 px drag
            // clears far more pixels than one dab.
            let cleared = (100..160).flat_map(|x| (120..146).map(move |y| (x, y))).filter(|&(x, y)| rgba(&h, x, y)[3] == 0.0).count();
            assert!(cleared >= 20, "{tool:?}: erased along the drag, {cleared} pixels");
            assert_eq!(last_stroke(h.state())["mode"], json!("clear"), "{tool:?}");
            assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some(label));
        }
        key(&mut h, Key::Backtick, false, Modifiers::NONE);
        h.run_steps(1);
        assert!(!h.state().held_erase, "{tool:?}: released");
        assert_eq!(h.state().ui.tool, tool, "the tool stays");
        if erases {
            drag_with(&mut h, Modifiers::NONE, 110.0, 140.0, 150.0, 140.0);
            assert_eq!(last_stroke_erases(&h), Some(false), "{tool:?}: paints again");
        }
    }
}

/// While `~` is held the Brush's Mode shows Clear, as in Photoshop.
#[test]
fn the_mode_shows_clear_while_tilde_is_held() {
    let mut h = harness(Tool::Brush);
    assert!(!crate::paint_mouse::clears(h.state(), Tool::Brush));
    key(&mut h, Key::Backtick, true, Modifiers::NONE);
    assert!(crate::paint_mouse::clears(h.state(), Tool::Brush));
    key(&mut h, Key::Backtick, false, Modifiers::NONE);
    h.run_steps(1);
    assert!(!crate::paint_mouse::clears(h.state(), Tool::Brush));
}

/// #3368 through the real key handling: holding Space (the momentary Hand) with a pending crop
/// keeps it pending, while V (another tool) commits it, as in Photoshop.
#[test]
fn a_held_key_keeps_a_pending_crop_and_another_tools_key_commits_it() {
    let mut h = harness(Tool::Crop);
    // Classic Mode resizes the frame from the dragged handle; the default image mode resizes about the centre.
    h.state_mut().ui.tool_options.crop.shield.classic_mode = true;
    h.state_mut().ui.extras.snap = false;
    h.run_steps(1);
    drag_with(&mut h, Modifiers::NONE, 400.0, 300.0, 300.0, 200.0);
    h.run_steps(2);
    assert!(crate::crop_ui::pending(h.state()));
    key(&mut h, Key::Space, true, Modifiers::NONE);
    h.run_steps(2);
    key(&mut h, Key::Space, false, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Crop);
    assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_doc::Size::new(400, 300));
    assert!(crate::crop_ui::pending(h.state()));
    tap(&mut h, Key::V, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Move);
    assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_doc::Size::new(300, 200));
}

/// Right-click while Space holds the Hand over another tool: the Hand tool's view menu, as in
/// Photoshop, not the held-over tool's menu; the tool comes back on release (#11271).
#[test]
fn space_right_click_opens_the_hand_view_menu() {
    // One app for the four tools: `harness` starts the whole app (its document, panels and settle frames), which is
    // most of a case, while the selected tool is state the app takes between frames (#15941). The tool is switched
    // the way the ⌘-Move test below does it.
    let mut h = harness(Tool::Move);
    for tool in [Tool::Move, Tool::Brush, Tool::RectMarquee, Tool::Pen] {
        h.state_mut().ui.tool = tool;
        // Every case starts at 100 %, not where the case before it left the shared view: the first
        // case ends at 200 %, and a later case whose row applied nothing would still read 200 %.
        let view = &mut h.state_mut().ui.views[0];
        view.zoom = 1.0;
        view.clear_pending();
        h.run_steps(1);
        let p = screen(&h, 200.0, 150.0);
        h.event(Event::PointerMoved(p));
        h.run_steps(1);
        key(&mut h, Key::Space, true, Modifiers::NONE);
        h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
        key(&mut h, Key::Space, false, Modifiers::NONE);
        h.run_steps(2);
        let menu = h.state().ui.canvas_tool_menu.clone().unwrap_or_else(|| panic!("{tool:?}: Space right-click opens no menu"));
        assert_eq!(menu.kind, MenuKind::View, "{tool:?}: the Hand's view menu, not the tool's or the layer list");
        assert_eq!(menu.under, Some(tool), "{tool:?}: held over the selected tool");
        assert_eq!(h.state().ui.tool, tool, "{tool:?}: the tool is kept");
        // Its rows work as the selected Hand's do.
        let before = zoom(&h);
        assert!(before != 2.0, "{tool:?}: the case starts away from 200 % ({before}), so the row has to move the view");
        let ctx = h.ctx.clone();
        crate::canvas_tool_menu::choose(h.state_mut(), &ctx, "view.twoHundredPercent");
        h.run_steps(2);
        assert!((zoom(&h) - 2.0).abs() < 1e-3, "{tool:?}: 200% from {before}");
        assert!(h.state().ui.canvas_tool_menu.is_none(), "{tool:?}: choosing closes the menu");
    }
}

/// The Hand's menu opened over another tool closes when that tool changes, as the selected
/// Hand's does when the Hand is put down (#11271).
#[test]
fn space_hand_view_menu_closes_when_the_tool_underneath_changes() {
    let mut h = harness(Tool::Brush);
    let p = screen(&h, 200.0, 150.0);
    h.event(Event::PointerMoved(p));
    h.run_steps(1);
    key(&mut h, Key::Space, true, Modifiers::NONE);
    h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    key(&mut h, Key::Space, false, Modifiers::NONE);
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_some(), "the menu outlives the Space release");
    h.state_mut().ui.tool = Tool::Lasso;
    h.run_steps(1);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "another tool closes it");
}

/// ⌘Space and ⌥Space hold the Zoom, not the Hand: a right-click opens no view menu (#11271).
/// With ⌥ it opens nothing, not even the held-over Move tool's layer list; with ⌘ it is still a
/// ⌘/Ctrl+right-click, which lists the layers under the pointer for any tool (#307).
#[test]
fn command_or_alt_space_right_click_opens_no_view_menu() {
    // One app for all four cases, as above; the menu a ⌘+right-click leaves open is closed before the next case.
    let mut h = harness(Tool::Move);
    for mods in [Modifiers::COMMAND, Modifiers::ALT] {
        for tool in [Tool::Move, Tool::Brush] {
            h.state_mut().ui.tool = tool;
            h.state_mut().ui.canvas_tool_menu = None;
            h.run_steps(1);
            let p = screen(&h, 200.0, 150.0);
            h.event(Event::PointerMoved(p));
            h.run_steps(1);
            key(&mut h, Key::Space, true, mods);
            h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: mods });
            h.run_steps(1);
            h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: mods });
            h.run_steps(1);
            key(&mut h, Key::Space, false, mods);
            h.run_steps(2);
            let kind = h.state().ui.canvas_tool_menu.as_ref().map(|m| m.kind.clone());
            assert_ne!(kind, Some(MenuKind::View), "{mods:?} {tool:?}: no view menu");
            let layers = matches!(kind, Some(MenuKind::Layers { .. }));
            assert_eq!(layers, mods.command, "{mods:?} {tool:?}: the layer list only for ⌘+right-click");
            assert_eq!(h.state().ui.tool, tool, "{mods:?} {tool:?}: the tool is kept");
        }
    }
}

/// A right-click at `p` with Space already held (the Hand over the selected tool).
fn right_click_at(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2) {
    h.event(Event::PointerMoved(p));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// A pending crop frame on the Crop tool, as the held-key test above sets one up: the bottom-right handle dragged in.
fn with_pending_crop(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    drag_with(h, Modifiers::NONE, 400.0, 300.0, 300.0, 200.0);
    h.run_steps(2);
    assert!(crate::crop_ui::pending(h.state()));
}

/// The Space-Hand view menu over a pending crop (#11271): Esc closes only the menu and Enter
/// commits nothing, as under the crop's own menu; with the menu closed Esc discards the crop.
#[test]
fn space_hand_view_menu_keeps_escape_and_enter_from_the_pending_crop() {
    let mut h = harness(Tool::Crop);
    with_pending_crop(&mut h);
    let size = h.state().session.active().map(|s| s.doc.size);
    let steps = h.state().session.active().map(|s| s.history.past_len());
    // The centre of the pending frame, as production's (200, 125) is of the frame it sets: the contextual task bar
    // hangs below the frame, so a point inside it is the canvas's.
    let centre = crate::crop_ui::frame(h.state()).expect("a pending crop frame").center();
    let p = screen(&h, centre.x as f32, centre.y as f32);
    key(&mut h, Key::Space, true, Modifiers::NONE);
    right_click_at(&mut h, p);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.kind == MenuKind::View), "Space's Hand opens the view menu");
    tap(&mut h, Key::Escape, Modifiers::NONE);
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "Esc closes the menu");
    assert!(crate::crop_ui::pending(h.state()), "and only the menu: the crop is not discarded");
    right_click_at(&mut h, p);
    assert!(h.state().ui.canvas_tool_menu.is_some(), "the view menu opens again");
    tap(&mut h, Key::Enter, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(h.state().session.active().map(|s| s.doc.size), size, "Enter under the menu doesn't crop");
    assert_eq!(h.state().session.active().map(|s| s.history.past_len()), steps, "no history step");
    assert!(crate::crop_ui::pending(h.state()));
    // Close the menu and let Space go: Esc discards the pending crop as ever.
    h.state_mut().ui.canvas_tool_menu = None;
    key(&mut h, Key::Space, false, Modifiers::NONE);
    h.run_steps(3);
    tap(&mut h, Key::Escape, Modifiers::NONE);
    h.run_steps(2);
    assert!(!crate::crop_ui::pending(h.state()), "with no menu open Esc discards the crop");
}

/// Esc with a tool key in one pass closes the view menu and picks the tool (#11271): the menu keeps only the
/// Esc, not the key pressed with it.
#[test]
fn space_hand_view_menu_reserves_only_plain_escape_and_enter() {
    let mut h = harness(Tool::Move);
    let p = screen(&h, 200.0, 125.0);
    key(&mut h, Key::Space, true, Modifiers::NONE);
    right_click_at(&mut h, p);
    assert!(h.state().ui.canvas_tool_menu.is_some(), "Space's Hand opens the view menu");
    h.input_mut().events.extend([
        Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        Event::Key { key: Key::B, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "Esc closes the menu");
    assert_eq!(h.state().ui.tool, Tool::Brush, "and B in the same pass still picks the Brush");
}

/// Space's Hand outside the image opens the pasteboard menu, which outlives the held Space and
/// closes when the tool underneath changes (#11271).
#[test]
fn space_hand_pasteboard_menu_opens_outside_the_image_and_outlives_space() {
    let mut h = harness(Tool::Move);
    let outside = screen(&h, -60.0, -60.0);
    key(&mut h, Key::Space, true, Modifiers::NONE);
    right_click_at(&mut h, outside);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.kind == MenuKind::Pasteboard), "Space's Hand opens the pasteboard menu outside");
    assert_eq!(h.state().ui.tool, Tool::Move, "the Move tool stays selected");
    key(&mut h, Key::Space, false, Modifiers::NONE);
    h.run_steps(3);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.kind == MenuKind::Pasteboard), "the pasteboard menu outlives the held Space");
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "another tool selected closes it");
}

/// A lone Alt tap or F10 puts the keyboard on the menu bar. Space then opens no menu (it used to open File, and a held
/// Space ran File > New) and is the Hand in the same press: the drag pans, as it always does in the reference
/// application (#16327).
#[test]
fn space_after_an_alt_tap_or_f10_pans_and_opens_no_menu() {
    for (f10, clicked) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut h = harness(Tool::Brush);
        let p = screen(&h, 200.0, 150.0);
        if clicked {
            // A dab first, so the canvas holds the keyboard when the bar takes it.
            click(&mut h, 100.0, 80.0, Modifiers::NONE);
        }
        h.event(Event::PointerMoved(p));
        h.run_steps(2);
        if f10 {
            key(&mut h, Key::F10, true, Modifiers::NONE);
            key(&mut h, Key::F10, false, Modifiers::NONE);
        } else {
            h.event(Event::ModifiersChanged(Modifiers::ALT));
            h.run_steps(1);
            h.event(Event::ModifiersChanged(Modifiers::NONE));
            h.run_steps(2);
        }
        assert!(craft::ui::menu::bar::is_focused(&h.ctx), "f10 {f10}, clicked {clicked}: the keyboard is on the bar");
        let before = center(&h);
        key(&mut h, Key::Space, true, Modifiers::NONE);
        assert!(!craft::ui::menu::bar::is_open(&h.ctx), "f10 {f10}, clicked {clicked}: Space opens no menu");
        assert!(!craft::ui::menu::bar::is_focused(&h.ctx), "f10 {f10}, clicked {clicked}: Space takes the keyboard off the bar");
        button(&mut h, p, true, Modifiers::NONE);
        move_to(&mut h, 250.0, 180.0);
        let q = screen(&h, 250.0, 180.0);
        button(&mut h, q, false, Modifiers::NONE);
        key(&mut h, Key::Space, false, Modifiers::NONE);
        h.run_steps(2);
        assert!(!craft::ui::menu::bar::is_open(&h.ctx), "f10 {f10}, clicked {clicked}: no menu after the drag");
        assert_ne!(center(&h), before, "f10 {f10}, clicked {clicked}: Space and a drag pan the view");
        assert_eq!(h.state().session.documents().len(), 1, "f10 {f10}, clicked {clicked}: no new document");
        assert_eq!(h.state().ui.tool, Tool::Brush, "f10 {f10}, clicked {clicked}: the tool comes back");
    }
}
