//! The pen's barrel (side) button as the right mouse button. On Windows with Windows Ink, winit
//! turns the pen into touch input and drops its buttons (#5364, #5905): a tap with the side button
//! held arrives as a left click, and the driver's emulated right-click (side switch set to Right
//! Click, pen hovering) as nothing at all. The desktop app reads the button beside winit
//! (`photocraft-tablet`, [`crate::Services::pen_barrel`]) and [`BarrelRightClick::apply`] rewrites
//! the frame's raw input before egui sees it, so every right-click works with the pen: context
//! menus (Layers panel, document tabs, text fields), the Brush Preset picker, ⌘/Ctrl+right-click
//! layer picking, Alt+right-drag brush resize and Right-click Erases.

use egui::{Event, Modifiers, PointerButton, Pos2};

/// What the pen's barrel button did since the last frame (the platform's report).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BarrelInput {
    /// Held now.
    pub held: bool,
    /// Went down since the last frame (it may be up again already).
    pub pressed: bool,
    /// Clicked since the last frame (a quick press and release: a driver's Right Click).
    pub clicked: bool,
    /// The pen's contact is one the tip never pressed (the driver's Right Click in the air).
    pub tipless: bool,
}

/// A pen wobbles while it taps: moves this close to a right press (in points) are held back, so
/// the tap stays a click (egui counts 6 points of movement as a drag). Farther, it is a right-drag.
const JITTER: f32 = 12.0;

/// A left press turned into a right one.
#[derive(Clone, Copy, Debug)]
struct RightPress {
    origin: Pos2,
    /// The pen went past [`JITTER`]: a right-drag, whose moves all pass.
    dragging: bool,
    /// Made in the air with the button alone (the tip never pressed): a click however far the pen
    /// moves, so all its moves are held back.
    tipless: bool,
}

/// Turns pen presses made with the barrel button into right presses (see the module docs).
#[derive(Clone, Debug, Default)]
pub(crate) struct BarrelRightClick {
    /// The press being turned, until its release.
    press: Option<RightPress>,
    /// The barrel button's current press already made a right press (a tap with it held), so its
    /// click is not a second right-click.
    used: bool,
    /// Where the pointer last was, for a click that comes without a press of its own.
    last_pos: Option<Pos2>,
}

impl BarrelRightClick {
    /// Rewrite one frame's raw `events` for what the barrel button did (`input`).
    pub(crate) fn apply(&mut self, input: BarrelInput, events: &mut Vec<Event>, modifiers: Modifiers) {
        if input.pressed {
            self.used = false;
        }
        let active = input.held || input.pressed;
        // The tip pressed after all: a tap, whose moves may make a right-drag again.
        if let Some(r) = self.press.as_mut() {
            r.tipless &= input.tipless;
        }
        let mut out = Vec::with_capacity(events.len() + 3);
        for e in events.drain(..) {
            match e {
                Event::PointerMoved(p) => {
                    self.last_pos = Some(p);
                    if let Some(r) = self.press.as_mut()
                        && !r.dragging
                    {
                        if r.tipless || (p - r.origin).length() <= JITTER {
                            continue;
                        }
                        r.dragging = true;
                    }
                    out.push(e);
                }
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers } if active && self.press.is_none() => {
                    self.press = Some(RightPress { origin: pos, dragging: false, tipless: input.tipless });
                    self.used = true;
                    self.last_pos = Some(pos);
                    out.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed: true, modifiers });
                }
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers } if let Some(r) = self.press => {
                    self.press = None;
                    // A tap lifts where it went down, however the pen wobbled.
                    let pos = if r.dragging { pos } else { r.origin };
                    out.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed: false, modifiers });
                }
                // The pointer left or the window lost focus with the press still down: it is over,
                // so a later, unrelated left release stays a left release.
                Event::PointerGone | Event::WindowFocused(false) => {
                    self.press = None;
                    out.push(e);
                }
                e => out.push(e),
            }
        }
        // The driver's Right Click: a click of the button with no press of the pen's own.
        if input.clicked
            && !self.used
            && self.press.is_none()
            && let Some(pos) = self.last_pos
        {
            out.push(Event::PointerMoved(pos));
            out.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed: true, modifiers });
            out.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed: false, modifiers });
        }
        if input.clicked && self.press.is_none() {
            self.used = false;
        }
        *events = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELD: BarrelInput = BarrelInput { held: true, pressed: true, clicked: false, tipless: false };
    const STILL_HELD: BarrelInput = BarrelInput { held: true, pressed: false, clicked: false, tipless: false };
    const NONE: BarrelInput = BarrelInput { held: false, pressed: false, clicked: false, tipless: false };
    const CLICK: BarrelInput = BarrelInput { held: false, pressed: true, clicked: true, tipless: false };

    fn button(x: f32, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos: egui::pos2(x, 50.0), button, pressed, modifiers: Modifiers::NONE }
    }

    fn moved(x: f32) -> Event {
        Event::PointerMoved(egui::pos2(x, 50.0))
    }

    fn run(b: &mut BarrelRightClick, input: BarrelInput, events: Vec<Event>) -> Vec<Event> {
        let mut events = events;
        b.apply(input, &mut events, Modifiers::NONE);
        events
    }

    #[test]
    fn a_tap_with_the_button_held_is_a_right_click() {
        let mut b = BarrelRightClick::default();
        let got = run(&mut b, HELD, vec![moved(50.0), button(50.0, PointerButton::Primary, true)]);
        assert_eq!(got, vec![moved(50.0), button(50.0, PointerButton::Secondary, true)]);
        // The wobble of the tap is held back and the release lands where the press did.
        let got = run(&mut b, STILL_HELD, vec![moved(55.0), button(55.0, PointerButton::Primary, false), Event::PointerGone]);
        assert_eq!(got, vec![button(50.0, PointerButton::Secondary, false), Event::PointerGone]);
        // Letting go of the button quickly afterwards is no second right-click.
        assert_eq!(run(&mut b, BarrelInput { held: false, pressed: false, clicked: true, tipless: false }, vec![]), vec![]);
        // The next click of the button alone is one.
        assert_eq!(run(&mut b, CLICK, vec![]).len(), 3);
    }

    #[test]
    fn a_drag_with_the_button_held_is_a_right_drag() {
        let mut b = BarrelRightClick::default();
        run(&mut b, HELD, vec![button(50.0, PointerButton::Primary, true)]);
        let got = run(&mut b, STILL_HELD, vec![moved(55.0), moved(80.0), moved(90.0), button(90.0, PointerButton::Primary, false)]);
        assert_eq!(got, vec![moved(80.0), moved(90.0), button(90.0, PointerButton::Secondary, false)]);
    }

    #[test]
    fn a_right_press_in_the_air_stays_a_click_however_far_the_pen_moves() {
        let in_air = BarrelInput { tipless: true, ..HELD };
        let mut b = BarrelRightClick::default();
        let got = run(&mut b, in_air, vec![button(50.0, PointerButton::Primary, true), moved(70.0)]);
        assert_eq!(got, vec![button(50.0, PointerButton::Secondary, true)]);
        let got = run(&mut b, BarrelInput { pressed: false, ..in_air }, vec![moved(120.0), moved(200.0), button(200.0, PointerButton::Primary, false)]);
        assert_eq!(got, vec![button(50.0, PointerButton::Secondary, false)], "no move passes; it lifts where it went down");
        assert_eq!(run(&mut b, NONE, vec![moved(210.0)]), vec![moved(210.0)], "later moves pass again");
    }

    #[test]
    fn a_right_press_whose_tip_presses_after_all_may_drag_again() {
        let mut b = BarrelRightClick::default();
        run(&mut b, BarrelInput { tipless: true, ..HELD }, vec![button(50.0, PointerButton::Primary, true)]);
        let got = run(&mut b, STILL_HELD, vec![moved(55.0), moved(80.0), button(80.0, PointerButton::Primary, false)]);
        assert_eq!(got, vec![moved(80.0), button(80.0, PointerButton::Secondary, false)]);
    }

    #[test]
    fn the_drivers_right_click_while_hovering_is_a_right_click_where_the_pen_is() {
        let mut b = BarrelRightClick::default();
        run(&mut b, NONE, vec![moved(70.0), Event::PointerGone]);
        let got = run(&mut b, CLICK, vec![]);
        assert_eq!(got, vec![moved(70.0), button(70.0, PointerButton::Secondary, true), button(70.0, PointerButton::Secondary, false)]);
        assert_eq!(run(&mut b, NONE, vec![]), vec![], "once");
    }

    #[test]
    fn without_the_button_a_tap_stays_a_left_click() {
        let mut b = BarrelRightClick::default();
        let events = vec![moved(50.0), button(50.0, PointerButton::Primary, true), moved(52.0), button(52.0, PointerButton::Primary, false)];
        assert_eq!(run(&mut b, NONE, events.clone()), events);
        // A press that began without the button stays a left press when the button goes down.
        let mut b = BarrelRightClick::default();
        assert_eq!(run(&mut b, NONE, vec![button(50.0, PointerButton::Primary, true)]), vec![button(50.0, PointerButton::Primary, true)]);
        assert_eq!(run(&mut b, HELD, vec![button(50.0, PointerButton::Primary, false)]), vec![button(50.0, PointerButton::Primary, false)]);
    }

    #[test]
    fn a_press_ends_when_the_pointer_leaves_or_the_window_loses_focus() {
        for end in [Event::PointerGone, Event::WindowFocused(false)] {
            let mut b = BarrelRightClick::default();
            run(&mut b, HELD, vec![button(50.0, PointerButton::Primary, true)]);
            assert_eq!(run(&mut b, NONE, vec![end.clone()]), vec![end.clone()], "passed on");
            // A later left release (the mouse, a new press) is not turned into a right one.
            assert_eq!(
                run(&mut b, NONE, vec![button(60.0, PointerButton::Primary, false)]),
                vec![button(60.0, PointerButton::Primary, false)],
                "after {end:?}"
            );
        }
    }

    #[test]
    fn a_real_right_button_passes_through() {
        let mut b = BarrelRightClick::default();
        let events = vec![button(50.0, PointerButton::Secondary, true), button(50.0, PointerButton::Secondary, false)];
        assert_eq!(run(&mut b, NONE, events.clone()), events);
    }

    /// End to end through egui: the pen's taps and clicks reach `secondary_clicked`.
    #[test]
    fn egui_sees_a_secondary_click() {
        let ctx = egui::Context::default();
        let mut b = BarrelRightClick::default();
        let mut right = 0;
        let mut left = 0;
        let frames: Vec<(BarrelInput, Vec<Event>)> = vec![
            (NONE, vec![]),
            (HELD, vec![moved(50.0), button(50.0, PointerButton::Primary, true)]),
            (STILL_HELD, vec![moved(54.0), button(54.0, PointerButton::Primary, false)]),
            (NONE, vec![]),
            (CLICK, vec![]),
            (NONE, vec![]),
            // The driver's Right Click in the air while the pen sweeps across.
            (BarrelInput { tipless: true, ..HELD }, vec![moved(60.0), button(60.0, PointerButton::Primary, true)]),
            (BarrelInput { tipless: true, ..STILL_HELD }, vec![moved(100.0), moved(150.0), button(150.0, PointerButton::Primary, false)]),
            (NONE, vec![]),
        ];
        for (i, (input, mut events)) in frames.into_iter().enumerate() {
            b.apply(input, &mut events, Modifiers::NONE);
            let mut raw = egui::RawInput {
                time: Some(i as f64 * 0.1),
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(200.0, 200.0))),
                ..Default::default()
            };
            raw.events = events;
            let mut out = ctx.run_ui(raw, |ui| {
                let r = ui.allocate_rect(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(200.0, 200.0)), egui::Sense::click_and_drag());
                right += usize::from(r.secondary_clicked());
                left += usize::from(r.clicked());
            });
            out.textures_delta.clear();
        }
        assert_eq!((right, left), (3, 0), "the tap with the button held, the driver's click and the one in the air");
    }
}
