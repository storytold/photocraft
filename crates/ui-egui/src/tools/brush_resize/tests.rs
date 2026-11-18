use craft::behavior::PointerButton;
use craft::keymap::Mods;
use craft::ui::runtime::windows::{Released, WindowHost};
use egui::Modifiers;
use serde_json::json;

use super::*;

use crate::canvas::testing;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn ctrl_alt() -> Modifiers {
    Modifiers { alt: true, ctrl: true, ..Default::default() }
}

fn alt() -> Modifiers {
    Modifiers { alt: true, ..Default::default() }
}

fn mods_of(m: Modifiers) -> Mods {
    Mods { cmd: false, ctrl: m.ctrl, alt: m.alt, shift: m.shift }
}

/// A 200 × 100 document, red on the left half and green on the right, the foreground blue and `tool`'s brush at 40 px.
fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": BLUE, "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
    // The tool's own brush first (a tool seen for the first time loads its starting tip, the Background Eraser's
    // 13 px), then the 40 px brush on it, as production's tests pick the tool before setting the brush.
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 0.2}})).unwrap();
    app.sync_views();
    // The Ctrl+Alt chord is a Mac gesture: off macOS the window host reports Ctrl as the command key.
    testing::on_a_mac(&app);
    app
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn painted_since(app: &PhotocraftApp, journal: usize) -> bool {
    app.session.journal[journal..].iter().any(|(id, _)| id == "paint.stroke" || id == "brush.mixer.sample" || id == "paint.mixerBrush")
}

/// Runs frames until the window host has delivered the queued input.
fn settle(app: &mut PhotocraftApp, host: &WindowHost) {
    for _ in 0..64 {
        testing::frame(app);
        if host.injected_idle() {
            return;
        }
    }
    panic!("the injected input was not taken");
}

/// One press, a drag to `to` in four steps and the release, with the given button and modifiers. The primary button
/// goes through the canvas driver; the secondary one through the window host's input, as the driver has no
/// secondary button.
fn drive(app: &mut PhotocraftApp, button: PointerButton, from: [f64; 2], to: [f64; 2], m: Modifiers) {
    let step = |k: f64| [from[0] + (to[0] - from[0]) * k / 4.0, from[1] + (to[1] - from[1]) * k / 4.0];
    if button == PointerButton::Primary {
        testing::press(app, from, m);
        if from != to {
            for k in 1..=4 {
                testing::move_to(app, step(f64::from(k)), m);
            }
        }
        testing::release(app, to, m);
        return;
    }
    testing::shown(app);
    let host = WindowHost::of(&testing::ctx(app));
    let pos = testing::to_screen(app, from);
    host.inject_click(egui::ViewportId::ROOT, pos, button, 1, mods_of(m), true);
    settle(app, &host);
    if from != to {
        for k in 1..=4 {
            let pos = testing::to_screen(app, step(f64::from(k)));
            host.inject_move(egui::ViewportId::ROOT, pos, mods_of(m), 0.0);
            settle(app, &host);
        }
    }
    let pos = testing::to_screen(app, to);
    host.inject_move(egui::ViewportId::ROOT, pos, mods_of(m), 0.0);
    host.inject_release(egui::ViewportId::ROOT, Some(pos), Released::Button(button));
    settle(app, &host);
}

#[derive(Clone, Copy, Debug)]
enum Outcome {
    /// The brush grows; nothing is painted, sampled or recorded.
    Resize,
    /// The foreground becomes the colour; nothing is painted or recorded.
    Sample([f32; 4]),
    /// The tool paints: a history step.
    Paint,
    /// The brush picker opens; nothing else happens.
    Picker,
}

struct Row {
    name: String,
    tool: Tool,
    button: PointerButton,
    mods: Modifiers,
    drag: bool,
    want: Outcome,
}

fn row(name: &str, tool: Tool, button: PointerButton, mods: Modifiers, drag: bool, want: Outcome) -> Row {
    Row { name: name.into(), tool, button, mods, drag, want }
}

fn rows() -> Vec<Row> {
    use PointerButton::{Primary, Secondary};
    let mut rows = Vec::new();
    // Control+Alt with the primary button resizes every brush tool and Quick Selection.
    let resizing: Vec<Tool> = Tool::ALL.into_iter().filter(|t| PaintResize::takes(*t, Primary, mods_of(ctrl_alt()))).collect();
    assert!(resizing.len() > 10 && resizing.contains(&Tool::Brush) && resizing.contains(&Tool::Eraser) && resizing.contains(&Tool::CloneStamp));
    assert!(resizing.contains(&Tool::QuickSelection) && resizing.contains(&Tool::Pencil) && resizing.contains(&Tool::BackgroundEraser));
    for tool in resizing {
        rows.push(row(&format!("ctrl+alt {tool:?}"), tool, Primary, ctrl_alt(), true, Outcome::Resize));
    }
    for tool in [Tool::Brush, Tool::MixerBrush, Tool::ColorReplacement, Tool::BackgroundEraser] {
        rows.push(row(&format!("alt+right {tool:?}"), tool, Secondary, alt(), true, Outcome::Resize));
    }
    // PLAN: the stylus barrel row needs a barrel-button injection the canvas harness lacks (`canvas/testing.rs`, canvas-widgets-w14).
    rows.push(row("plain right click", Tool::Brush, Secondary, Modifiers::NONE, false, Outcome::Picker));
    // The Background Eraser: Alt samples, a plain click erases.
    rows.push(row("alt click background eraser", Tool::BackgroundEraser, Primary, alt(), false, Outcome::Sample(GREEN)));
    rows.push(row("alt drag background eraser", Tool::BackgroundEraser, Primary, alt(), true, Outcome::Sample(RED)));
    rows.push(row("plain click background eraser", Tool::BackgroundEraser, Primary, Modifiers::NONE, false, Outcome::Paint));
    // Color Replacement: Alt click and drag pick the foreground, a plain click paints.
    rows.push(row("alt click color replacement", Tool::ColorReplacement, Primary, alt(), false, Outcome::Sample(GREEN)));
    rows.push(row("alt drag color replacement", Tool::ColorReplacement, Primary, alt(), true, Outcome::Sample(RED)));
    rows.push(row("plain click color replacement", Tool::ColorReplacement, Primary, Modifiers::NONE, false, Outcome::Paint));
    rows
}

/// Three handlers compete for one press: the resize chord, the Alt eyedropper and the picker on a right click. Each
/// row says which one wins, and for a resize or a sample that nothing is painted and no history step is recorded.
#[test]
fn resize_chords_and_alt_sampling_route_by_tool() {
    for r in rows() {
        let mut a = app(r.tool);
        let (h0, j0, size0, mixer0) = (history(&a), a.session.journal.len(), a.session.tools.brush.size, a.session.tools.mixer.clone());
        // Resize and picker rows press where the pointer drags right; sample rows press on green and drag to red.
        let (from, to) = match r.want {
            Outcome::Sample(_) => ([150.0, 50.0], if r.drag { [40.0, 50.0] } else { [150.0, 50.0] }),
            _ => ([100.0, 50.0], if r.drag { [130.0, 50.0] } else { [100.0, 50.0] }),
        };
        drive(&mut a, r.button, from, to, r.mods);
        let name = &r.name;
        assert!(!a.brush_resize.active(), "{name}: the resize ended with the button");
        assert!(!testing::in_gesture(&a), "{name}: the gesture ended");
        assert_eq!(a.session.tools.mixer, mixer0, "{name}: the mixer is untouched");
        match r.want {
            Outcome::Resize => {
                assert!(a.session.tools.brush.size > size0, "{name}: right grows the brush");
                assert_eq!(history(&a), h0, "{name}: no history step");
                assert!(!painted_since(&a, j0), "{name}: nothing painted");
                assert_eq!(a.session.tools.foreground, BLUE, "{name}: nothing sampled");
                assert!(a.ui.brush_picker.is_none(), "{name}: no picker");
            }
            Outcome::Sample(color) => {
                assert_eq!(a.session.tools.foreground, color, "{name}: the foreground follows the pointer");
                assert_eq!(history(&a), h0, "{name}: no history step");
                assert!(!painted_since(&a, j0), "{name}: nothing painted");
                assert_eq!(a.session.tools.brush.size, size0, "{name}: no resize");
            }
            Outcome::Paint => {
                assert!(history(&a) > h0, "{name}: the tool painted");
                assert_eq!(a.session.tools.brush.size, size0, "{name}: no resize");
                assert_eq!(a.session.tools.foreground, BLUE, "{name}: nothing sampled");
            }
            Outcome::Picker => {
                assert!(a.ui.brush_picker.is_some(), "{name}: the picker opened");
                assert_eq!(history(&a), h0, "{name}: no history step");
                assert_eq!(a.session.tools.brush.size, size0, "{name}: no resize");
            }
        }
    }
}

#[test]
fn other_tools_and_strokes_in_progress_are_untouched() {
    use PointerButton::Primary;
    assert!(!PaintResize::takes(Tool::Move, Primary, mods_of(ctrl_alt())), "the Move tool has no brush");
    let mut a = app(Tool::Move);
    let j0 = a.session.journal.len();
    drive(&mut a, Primary, [10.0, 10.0], [10.0, 10.0], ctrl_alt());
    assert!(!a.brush_resize.active());
    assert!(!a.session.journal[j0..].iter().any(|(id, _)| id == "tools.setBrush"), "no resize reached the brush");
    // A stroke in progress is not taken over by the chord.
    let mut a = app(Tool::Brush);
    testing::press(&mut a, [10.0, 10.0], Modifiers::NONE);
    testing::move_to(&mut a, [50.0, 10.0], ctrl_alt());
    assert!(!a.brush_resize.active(), "a stroke in progress isn't taken over");
    testing::release(&mut a, [50.0, 10.0], ctrl_alt());
    assert!(a.session.journal.iter().any(|(id, _)| id == "paint.stroke"));
    assert_eq!(a.session.tools.brush.size, 40.0);
}
