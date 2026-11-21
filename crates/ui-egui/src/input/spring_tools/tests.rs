//! #2672 through the real key handling and canvas: a tapped tool letter switches for good, a held
//! one (or one used on the canvas) springs back on release.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::*;

/// Frames are 1/60 s apart, so a hold is a count of frames.
const DT: f32 = 1.0 / 60.0;

/// The whole app with a 400×300 document at 100 %: the runtime takes the keys of a frame and reports the fired
/// commands to the app in the next one, so every key event runs two frames (`key_event`).
fn harness(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(DT).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        app.ui.extras.rulers = false;
        app.ui.tool = tool;
        app
    });
    h.run_steps(6);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.clear_pending();
    h.run_steps(2);
    h
}

/// The screen point of the document point (`x`, `y`) through the view the canvas shows.
fn screen(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32) -> Pos2 {
    crate::canvas::testing::to_screen(h.state_mut(), [f64::from(x), f64::from(y)])
}

/// One key event with its modifiers, then two frames: the pass that takes the key and the one that runs its command.
fn key_event(h: &mut Harness<'static, Box<PhotocraftApp>>, k: Key, pressed: bool, repeat: bool, m: Modifiers) {
    h.event(Event::ModifiersChanged(m));
    h.event(Event::Key { key: k, physical_key: None, pressed, repeat, modifiers: m });
    h.run_steps(2);
}

fn down(h: &mut Harness<'static, Box<PhotocraftApp>>, k: Key) {
    key_event(h, k, true, false, Modifiers::NONE);
}

fn up(h: &mut Harness<'static, Box<PhotocraftApp>>, k: Key) {
    key_event(h, k, false, false, Modifiers::NONE);
    h.run_steps(2);
}

/// Wait `secs` of input time.
fn wait(h: &mut Harness<'static, Box<PhotocraftApp>>, secs: f64) {
    h.run_steps((secs / f64::from(DT)).ceil() as usize);
}

fn button(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, pressed: bool, m: Modifiers) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: m });
    h.run_steps(1);
}

fn move_to(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(Event::PointerMoved(p));
    h.run_steps(1);
}

fn click(h: &mut Harness<'static, Box<PhotocraftApp>>, x: f32, y: f32, m: Modifiers) {
    move_to(h, x, y);
    let p = screen(h, x, y);
    button(h, p, true, m);
    button(h, p, false, m);
}

#[test]
fn a_tap_switches_tools_for_good() {
    let mut h = harness(Tool::Brush);
    down(&mut h, Key::E);
    assert_eq!(h.state().ui.tool, Tool::Eraser);
    assert!(h.state().ui.spring.is_some(), "the press is pending: tap or hold");
    up(&mut h, Key::E);
    assert_eq!(h.state().ui.tool, Tool::Eraser);
    assert_eq!(h.state().ui.spring, None);
}

#[test]
fn a_held_letter_gives_the_previous_tool_back() {
    let mut h = harness(Tool::Brush);
    down(&mut h, Key::E);
    wait(&mut h, SPRING_HOLD_SECS + 0.1);
    assert_eq!(h.state().ui.tool, Tool::Eraser, "the temporary tool is the tool shown meanwhile");
    up(&mut h, Key::E);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    assert_eq!(h.state().ui.spring, None);
}

#[test]
fn hold_z_click_zooms_then_a_quick_release_restores() {
    let mut h = harness(Tool::Brush);
    let z0 = h.state().ui.views[0].zoom;
    down(&mut h, Key::Z);
    click(&mut h, 200.0, 150.0, Modifiers::NONE);
    up(&mut h, Key::Z);
    let z1 = h.state().ui.views[0].zoom;
    assert!(z1 > z0, "{z0} -> {z1}");
    assert_eq!(h.state().ui.tool, Tool::Brush, "a canvas click makes even a short hold temporary");
    assert!(!h.state().session.active().unwrap().history.can_undo(), "nothing was painted");
    // ⌥ with a held Z zooms out, as ⌥ does with the Zoom tool.
    down(&mut h, Key::Z);
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    click(&mut h, 200.0, 150.0, Modifiers::ALT);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    up(&mut h, Key::Z);
    assert!(h.state().ui.views[0].zoom < z1);
    assert_eq!(h.state().ui.tool, Tool::Brush);
}

#[test]
fn shift_letter_cycles_and_never_springs() {
    let mut h = harness(Tool::Brush);
    h.state_mut().session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = true);
    key_event(&mut h, Key::B, true, false, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    assert_eq!(h.state().ui.spring, None);
    wait(&mut h, SPRING_HOLD_SECS + 0.1);
    key_event(&mut h, Key::B, false, false, Modifiers::SHIFT);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Pencil);

    // Without the preference ⇧E is still a permanent switch, however long it is held.
    let mut h = harness(Tool::Brush);
    h.state_mut().session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    key_event(&mut h, Key::E, true, false, Modifiers::SHIFT);
    wait(&mut h, SPRING_HOLD_SECS + 0.1);
    key_event(&mut h, Key::E, false, false, Modifiers::SHIFT);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Eraser);
    assert_eq!(h.state().ui.spring, None);
}

#[test]
fn a_drag_keeps_the_temporary_tool_until_the_button_is_up() {
    let mut h = harness(Tool::Brush);
    down(&mut h, Key::M);
    assert_eq!(h.state().ui.tool, Tool::RectMarquee);
    move_to(&mut h, 50.0, 50.0);
    let p = screen(&mut h, 50.0, 50.0);
    button(&mut h, p, true, Modifiers::NONE);
    move_to(&mut h, 100.0, 90.0);
    up(&mut h, Key::M);
    assert_eq!(h.state().ui.tool, Tool::RectMarquee, "the drag finishes with the temporary tool");
    move_to(&mut h, 150.0, 110.0);
    let p = screen(&mut h, 150.0, 110.0);
    button(&mut h, p, false, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    assert_eq!(h.state().ui.spring, None);
    let sel = h.state().session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
    assert!(sel.is_some_and(|r| r.width() > 50 && r.height() > 30), "{sel:?}");
}

#[test]
fn the_newest_letter_wins_and_release_restores_the_original() {
    let mut h = harness(Tool::Brush);
    down(&mut h, Key::E);
    down(&mut h, Key::L);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    up(&mut h, Key::E);
    assert_eq!(h.state().ui.tool, Tool::Lasso, "L is still held");
    up(&mut h, Key::L);
    assert_eq!(h.state().ui.tool, Tool::Brush);
}

/// A tool picked another way while a letter is held (here, as the toolbar would) stands: the release
/// doesn't take it back.
#[test]
fn a_tool_picked_while_a_letter_is_held_stands() {
    let mut h = harness(Tool::Brush);
    down(&mut h, Key::E);
    wait(&mut h, SPRING_HOLD_SECS + 0.1);
    h.state_mut().ui.tool = Tool::Lasso;
    up(&mut h, Key::E);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    assert_eq!(h.state().ui.spring, None);
}
