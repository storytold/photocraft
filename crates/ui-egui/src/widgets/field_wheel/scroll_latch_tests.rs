//! #11353: a panel being scrolled keeps scrolling when a control it brings under the resting
//! pointer would take the wheel (a value field, a dropdown, a slider), across the pause between
//! two trackpad swipes. Moving the pointer, or a longer pause, gives the wheel back to the control.

use egui::accesskit::Role;
use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[derive(Clone, Copy, Debug)]
enum Control {
    Field,
    Dropdown,
    Slider,
}

const ALL: [Control; 3] = [Control::Field, Control::Dropdown, Control::Slider];

const OPTIONS: [(usize, &str); 5] = [(0, "One"), (1, "Two"), (2, "Three"), (3, "Four"), (4, "Five")];

/// The field's or slider's value, the dropdown's choice, and the panel's scroll offset.
#[derive(Clone, Copy, Debug, PartialEq)]
struct State {
    value: f32,
    choice: usize,
    offset: f32,
}

const START: State = State { value: 500.0, choice: 2, offset: 0.0 };

/// A 100 pt tall panel with `control` 40 pt down (all of it in view), above a large filler; frames 1/30 s apart.
fn harness(control: Control) -> Harness<'static, State> {
    let mut h = Harness::builder().with_size(vec2(320.0, 240.0)).with_step_dt(1.0 / 30.0).build_ui_state(
        move |ui, s: &mut State| {
            super::begin_frame(ui.ctx());
            let out = egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
                ui.allocate_space(vec2(200.0, 40.0));
                match control {
                    Control::Field => {
                        crate::widgets::value_field(ui, &mut s.value, 0.0..=1000.0, "px", 90.0).accessible_name("Size");
                    }
                    Control::Dropdown => {
                        crate::widgets::dropdown_wheel_hovered(ui, "mode", "Mode", &mut s.choice, &OPTIONS, 120.0);
                    }
                    Control::Slider => {
                        let g = crate::widgets::RowGestures { wheel_step: Some(1.0), ..Default::default() };
                        crate::widgets::slider_row_with(ui, "Amount", &mut s.value, 0.0..=1000.0, "", None, g);
                    }
                }
                ui.allocate_space(vec2(200.0, 1000.0));
            });
            s.offset = out.state.clamped_offset().y;
        },
        START,
    );
    h.run_steps(4);
    h
}

fn rect(h: &Harness<'static, State>, control: Control) -> Rect {
    let role = match control {
        Control::Field => Role::SpinButton,
        Control::Dropdown => Role::ComboBox,
        Control::Slider => Role::Slider,
    };
    h.get_by_role(role).rect()
}

/// One trackpad-style wheel event of `dy` points (down negative), no modifiers.
fn swipe(h: &mut Harness<'static, State>, dy: f32) {
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
}

/// A first swipe with the pointer 40 pt above `control` scrolls the panel until the control
/// rests under the pointer, then the fingers pause for `pause` frames. Returns the pointer.
fn scroll_onto(h: &mut Harness<'static, State>, control: Control, pause: usize) -> Pos2 {
    let at = rect(h, control).center() - vec2(0.0, 40.0);
    h.hover_at(at);
    h.run_steps(1);
    swipe(h, -40.0);
    h.run_steps(pause);
    assert!(rect(h, control).contains(at), "{control:?}: the panel scrolled the control under the pointer");
    assert_eq!(h.state().value, START.value, "{control:?}: the first swipe started over the panel");
    assert_eq!(h.state().choice, START.choice);
    at
}

fn untouched(s: &State) -> bool {
    s.value == START.value && s.choice == START.choice
}

#[test]
fn the_next_swipe_of_a_panel_scroll_scrolls_past_a_control_under_the_resting_pointer() {
    for control in ALL {
        let mut h = harness(control);
        // A pause longer than one gesture's gap (0.3 s): the fingers rest between two swipes.
        scroll_onto(&mut h, control, 18);
        let offset = h.state().offset;
        assert!(offset > 0.0, "{control:?}: the first swipe scrolled the panel");
        swipe(&mut h, -40.0);
        h.run_steps(18);
        assert!(untouched(h.state()), "{control:?}: the control the panel brought under the pointer kept its value: {:?}", h.state());
        assert!(h.state().offset > offset, "{control:?}: the panel kept scrolling");
    }
}

#[test]
fn moving_the_pointer_gives_the_wheel_back_to_the_control_under_it() {
    for control in ALL {
        let mut h = harness(control);
        let at = scroll_onto(&mut h, control, 18);
        let offset = h.state().offset;
        h.hover_at(at + vec2(2.0, 0.0));
        h.run_steps(1);
        swipe(&mut h, -40.0);
        h.run_steps(18);
        assert!(!untouched(h.state()), "{control:?}: the wheel stepped the control under the moved pointer");
        assert_eq!(h.state().offset, offset, "{control:?}: the panel under it didn't scroll");
    }
}

#[test]
fn a_long_pause_ends_the_panel_scroll() {
    for control in ALL {
        let mut h = harness(control);
        // Two seconds, past the 1.5 s latch, with the pointer at rest.
        scroll_onto(&mut h, control, 60);
        let offset = h.state().offset;
        swipe(&mut h, -40.0);
        h.run_steps(18);
        assert!(!untouched(h.state()), "{control:?}: a new gesture over the control steps it");
        assert_eq!(h.state().offset, offset, "{control:?}: the panel under it didn't scroll");
    }
}

/// Without the fix a dropdown or slider took the wheel whenever the pointer was over it, even in
/// the middle of a panel scroll.
#[test]
fn a_swipe_that_starts_over_the_panel_does_not_step_a_dropdown_or_slider_it_reaches() {
    // A short swipe over the panel, then (with `second`) a whole notch more once the pointer has
    // reached the control, in the same gesture.
    let run = |control: Control, second: bool| {
        let mut h = harness(control);
        let at = rect(&h, control).center() - vec2(0.0, 40.0);
        h.hover_at(at);
        h.run_steps(1);
        swipe(&mut h, -10.0);
        h.run_steps(1);
        h.hover_at(rect(&h, control).center());
        if second {
            swipe(&mut h, -40.0);
        }
        h.run_steps(18);
        *h.state()
    };
    for control in ALL {
        let without = run(control, false);
        let with = run(control, true);
        assert!(untouched(&with), "{control:?}: {with:?}");
        // Against the first swipe alone, so its own scroll can't pass for the second's.
        assert!(with.offset > without.offset, "{control:?}: the panel kept scrolling: {} after {}", with.offset, without.offset);
    }
}

/// A slider or dropdown takes ⌥/Alt and ⌘/Ctrl + wheel too (Photoshop's sliders step by 1 with
/// them), where a value field leaves it to the view; a panel scroll still passes over them.
#[test]
fn a_slider_or_dropdown_takes_the_wheel_with_alt_or_ctrl() {
    for control in [Control::Dropdown, Control::Slider] {
        for modifiers in [egui::Modifiers::ALT, egui::Modifiers::CTRL] {
            let mut h = harness(control);
            h.hover_at(rect(&h, control).center());
            h.run_steps(1);
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, -1.0),
                phase: egui::TouchPhase::Move,
                modifiers,
            });
            h.run_steps(18);
            let s = *h.state();
            assert!(!untouched(&s), "{control:?} {modifiers:?}: stepped");
            assert_eq!(s.offset, 0.0, "{control:?} {modifiers:?}: the panel didn't scroll");
        }
    }
}

/// A wheel over the panel, `idle` frames (1/30 s each) later the pointer moves onto `control`,
/// and a frame after that a whole notch down: both within one gesture's 0.3 s.
fn move_then_wheel(control: Control, idle: usize) -> State {
    let mut h = harness(control);
    h.hover_at(rect(&h, control).center() - vec2(0.0, 30.0));
    h.run_steps(1);
    // Up, at the top already: the panel doesn't move, the gesture is the panel's.
    swipe(&mut h, 40.0);
    h.run_steps(1 + idle);
    h.hover_at(rect(&h, control).center());
    h.run_steps(1);
    swipe(&mut h, -40.0);
    h.run_steps(18);
    *h.state()
}

/// Moving the pointer more than 100 ms after the panel's last wheel is going somewhere: the
/// control it reaches takes the next wheel, though that comes within the gesture's 0.3 s.
#[test]
fn a_move_after_100_ms_lets_the_control_take_the_next_wheel() {
    for control in ALL {
        // The move 133 ms after the wheel, the next wheel at 167 ms.
        let s = move_then_wheel(control, 3);
        assert!(!untouched(&s), "{control:?}: the control under the moved pointer stepped: {s:?}");
        assert_eq!(s.offset, 0.0, "{control:?}: the panel didn't scroll");
    }
}

/// A move within 100 ms of the panel's last wheel is the hand drifting as it scrolls: the panel
/// keeps the wheel over the control the pointer drifted onto.
#[test]
fn a_move_within_100_ms_of_the_wheel_keeps_the_panel_scrolling() {
    for control in ALL {
        // The move 33 ms after the wheel, the next wheel at 67 ms.
        let s = move_then_wheel(control, 0);
        assert!(untouched(&s), "{control:?}: {s:?}");
        assert!(s.offset > 0.0, "{control:?}: the panel scrolled");
    }
}

/// The move and the next wheel in one frame, 133 ms after the panel's last wheel: the move is
/// going somewhere, so the control it reaches takes that wheel.
#[test]
fn a_move_in_the_wheels_own_frame_after_100_ms_lets_the_control_take_it() {
    for control in ALL {
        let mut h = harness(control);
        h.hover_at(rect(&h, control).center() - vec2(0.0, 30.0));
        h.run_steps(1);
        swipe(&mut h, 40.0);
        h.run_steps(4);
        // Straight into the next frame's input: queued events would each get a frame of their own.
        let at = rect(&h, control).center();
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.input_mut().events.push(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -40.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(18);
        let s = *h.state();
        assert!(!untouched(&s), "{control:?}: the control under the moved pointer stepped: {s:?}");
        assert_eq!(s.offset, 0.0, "{control:?}: the panel didn't scroll");
    }
}

/// The wheel and then the move in one frame, 133 ms after the panel's last wheel: the move came
/// after that wheel, so the wheel is still the panel's and the control it lands on stays as it is.
#[test]
fn a_move_after_the_wheel_in_its_frame_keeps_the_wheel_the_panels() {
    for control in ALL {
        let mut h = harness(control);
        h.hover_at(rect(&h, control).center() - vec2(0.0, 30.0));
        h.run_steps(1);
        swipe(&mut h, 40.0);
        h.run_steps(4);
        let at = rect(&h, control).center();
        h.input_mut().events.push(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -40.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.run_steps(18);
        let s = *h.state();
        assert!(untouched(&s), "{control:?}: {s:?}");
        assert!(s.offset > 0.0, "{control:?}: the panel scrolled");
    }
}
