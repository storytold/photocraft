//! The Pencil tool (#213): aliased strokes through the live-stroke path, Auto Erase, tip-shaped
//! cursor, ⇧-click lines, Control+Alt resizing and the B group.

use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80, "background": "transparent"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 3, "hardness": 0.0}})).unwrap();
    // Each tool keeps its own brush (#218): switch first, then set the Pencil's options, as the
    // options bar does (its first frame takes the Brush's tip at Photoshop's 10 % smoothing).
    app.ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"smoothing": {"amount": 0.0}}})).unwrap();
    app
}

fn layer(app: &PhotocraftApp) -> photocraft_engine::doc::Layer {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
}

fn rgba(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    layer(app).surface().unwrap().rgba(x, y)
}

fn drag(app: &mut PhotocraftApp, pts: &[(f64, f64)], mods: Modifiers) {
    let (x, y) = pts[0];
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    for &(x, y) in &pts[1..] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
    }
    let &(x, y) = pts.last().unwrap();
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

fn last(app: &PhotocraftApp) -> (String, serde_json::Value) {
    app.session.journal.last().cloned().unwrap()
}

#[test]
fn pencil_strokes_are_aliased_live_and_one_undo_step() {
    let mut app = app();
    let pts = [(10.0, 10.0), (40.3, 22.7), (70.0, 50.0), (100.0, 30.0)];
    let (x, y) = pts[0];
    tool_event(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    for &(x, y) in &pts[1..] {
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE);
    }
    // The canvas shows the engine's real dabs while dragging.
    assert!(app.live_stroke.is_some(), "the Pencil strokes live");
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 30.0 }, Modifiers::NONE);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.pencil");
    assert_eq!(p["autoErase"], json!(false));
    assert_eq!(app.session.active().unwrap().history.past_len(), 1);
    // 8 bits, a soft session brush: still no partial alpha anywhere.
    let s = layer(&app).surface().unwrap().clone();
    let mut painted = 0;
    for y in 0..80 {
        for x in 0..120 {
            let a = s.rgba(x, y)[3];
            assert!(a == 0.0 || a == 1.0, "partial alpha {a} at ({x},{y})");
            painted += usize::from(a == 1.0);
        }
    }
    assert!(painted > 200, "{painted}");
    assert_eq!(rgba(&app, 70, 50), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn auto_erase_paints_the_background_colour_from_foreground_pixels() {
    let mut app = app();
    drag(&mut app, &[(10.0, 40.0), (110.0, 40.0)], Modifiers::NONE);
    app.ui.tool_options.pencil_auto_erase = true;
    // Starting on the black line: white.
    drag(&mut app, &[(50.0, 40.0), (50.0, 70.0)], Modifiers::NONE);
    assert_eq!(rgba(&app, 50, 60), [1.0; 4]);
    assert_eq!(last(&app).1["autoErase"], json!(true));
    // Starting off it: black.
    drag(&mut app, &[(80.0, 10.0), (80.0, 30.0)], Modifiers::NONE);
    assert_eq!(rgba(&app, 80, 20), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn shift_click_draws_a_pencil_line() {
    let mut app = app();
    drag(&mut app, &[(10.0, 10.0), (20.0, 10.0)], Modifiers::NONE);
    drag(&mut app, &[(100.0, 60.0)], Modifiers::SHIFT);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.pencil");
    assert_eq!(p["points"].as_array().unwrap().len(), 2, "{p}");
    assert_eq!(rgba(&app, 60, 35)[3], 1.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), 2);
}

#[test]
fn ctrl_alt_drag_resizes_the_pencil_without_painting() {
    let mut app = app();
    let ctrl_alt = Modifiers { alt: true, ctrl: true, ..Default::default() };
    assert!(crate::brush_resize::applies(Tool::Pencil));
    drag(&mut app, &[(50.0, 40.0), (60.0, 40.0)], ctrl_alt);
    assert_eq!(app.session.tools.brush.size, 23.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), 0);
}

#[test]
fn b_cycles_brush_pencil_and_mixer_brush() {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app
    });
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(4);
    let press = |h: &mut Harness<'_, PhotocraftApp>, m: Modifiers| {
        h.event(egui::Event::Key { key: Key::B, physical_key: None, pressed: true, repeat: false, modifiers: m });
        h.event(egui::Event::Key { key: Key::B, physical_key: None, pressed: false, repeat: false, modifiers: m });
        h.run_steps(2);
    };
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::MixerBrush);
    // Use Shift Key for Tool Switch: ⇧B cycles, B keeps the group's tool.
    h.state_mut().session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = true);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::MixerBrush);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::MixerBrush);
    assert_eq!(Tool::from_name("pencil"), Some(Tool::Pencil));
    assert_eq!(Tool::from_name("mixerBrush"), Some(Tool::MixerBrush));
}
