//! The secondary button on the canvas under the paint tools: the HUD Color Picker's gesture and the
//! Shift+right-click menus of the painting tools.

use super::*;

/// Neither the Brush Preset picker nor a canvas menu (the mode menu included) is open.
fn nothing_opened(h: &Harness<'static, Box<PhotocraftApp>>) -> bool {
    picker_at(h).is_none() && h.state().ui.canvas_tool_menu.is_none()
}

/// One pointer button event in its own frame.
fn button(h: &mut Harness<'static, Box<PhotocraftApp>>, pos: Pos2, button: PointerButton, pressed: bool) {
    h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// A pointer move by `by` from `from`, in `steps` frames; returns where it ends.
fn move_by(h: &mut Harness<'static, Box<PhotocraftApp>>, from: Pos2, by: egui::Vec2, steps: usize) -> Pos2 {
    for i in 1..=steps {
        h.event(egui::Event::PointerMoved(from + by * (i as f32 / steps as f32)));
        h.run_steps(1);
    }
    from + by
}

/// One key going down and up, each in its own frame, with `mods` held.
fn key(h: &mut Harness<'static, Box<PhotocraftApp>>, key: egui::Key, mods: Modifiers) {
    for pressed in [true, false] {
        h.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods });
        h.run_steps(1);
    }
}

/// Escape pressed while the Shift+Alt right button is down closes the HUD Color Picker it opened;
/// the release after it paints nothing and opens no menu.
#[test]
fn escape_while_the_right_button_is_down_cancels_the_right_click() {
    let mods = Modifiers::SHIFT | Modifiers::ALT;
    let mut h = harness(None);
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::ModifiersChanged(mods));
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: c, button: PointerButton::Secondary, pressed: true, modifiers: mods });
    h.run_steps(1);
    assert!(crate::color_hud::is_open(&h.state().ui), "the HUD opens for Shift+Alt");
    let p = move_by(&mut h, c, vec2(8.0, 0.0), 2);
    key(&mut h, egui::Key::Escape, mods);
    assert!(!crate::color_hud::is_open(&h.state().ui), "Escape closes the HUD");
    let p = move_by(&mut h, p, vec2(6.0, 4.0), 2);
    button(&mut h, p, PointerButton::Secondary, false);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    assert!(nothing_opened(&h), "Escape cancelled the right-click");
    assert!(strokes(&h).is_empty(), "and nothing paints");
    // The next right-click is one again.
    mode_right_click(&mut h, Modifiers::NONE);
    assert!(!nothing_opened(&h), "the next right-click opens its menu");
}

/// The Shift+Alt+right HUD Color Picker opens with the current colour under the pointer. A press
/// that opened it is the HUD's gesture: released after a small move, on the HUD or just off it
/// on the canvas, it closes the HUD and opens neither the Brush Preset picker nor a canvas menu.
#[test]
fn a_hud_color_picker_press_moved_a_few_pixels_opens_no_menu() {
    let shift_alt = Modifiers::SHIFT | Modifiers::ALT;
    // The harness paints in blue, the HUD square's top-right corner: up is off the HUD, down
    // and left is on it.
    for by in [vec2(0.0, -10.0), vec2(-10.0, 10.0)] {
        let mut h = harness(None);
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::ModifiersChanged(shift_alt));
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: c, button: PointerButton::Secondary, pressed: true, modifiers: shift_alt });
        h.run_steps(2);
        assert!(crate::color_hud::is_open(&h.state().ui), "{by:?}: the HUD opens");
        let p = move_by(&mut h, c, by, 2);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: shift_alt });
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
        assert!(!crate::color_hud::is_open(&h.state().ui), "{by:?}: the release closes the HUD");
        assert!(nothing_opened(&h), "{by:?}: and opens no menu");
        assert!(strokes(&h).is_empty(), "{by:?}");
    }
}
