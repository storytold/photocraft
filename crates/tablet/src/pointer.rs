//! Windows `WM_POINTER` pen frames → synthesized mouse messages and [`Signal`]s. Pure (no Windows
//! calls), so it is tested on every platform; `windows.rs` reads each frame's
//! `POINTER_INFO`/`POINTER_PEN_INFO` and feeds it here.
//!
//! winit 0.30 reports a pen only as `WindowEvent::Touch` (pressure via touch force) and lets egui
//! emulate the mouse from it: the frame's buttons, its cancel flag and the tilt/rotation/eraser
//! fields never reach the UI, and egui pins its emulated button to the first touch id. So the
//! pieces below make the pen a mouse: a tip contact becomes move + left-button messages, the
//! barrel button a real right click or drag, and a cancelled contact ([`Signal::Cancel`]) tells
//! the UI to drop the stroke it had begun. `windows.rs` posts the messages.
//!
//! Windows Ink's right clicks are *system gestures*: a press-and-hold (or a right tap whose frames
//! carried no button) cancels the tip contact (`POINTER_FLAG_CANCELED`) and posts
//! `WM_CONTEXTMENU`, whose right click [`gesture`] describes. The two must not fire twice: a
//! gesture right after a contact that already used the right button is the system's echo of it.

use crate::{Sample, Signal};

/// The pen-related fields of one pointer frame (`POINTER_INFO` + `POINTER_PEN_INFO`), read as
/// plain values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawFrame {
    /// The pen is touching the surface (`POINTER_FLAG_INCONTACT`).
    pub in_contact: bool,
    /// The system cancelled the contact (`POINTER_FLAG_CANCELED`): a press-and-hold right click
    /// fired and this is the gesture's retracted tip contact.
    pub cancelled: bool,
    /// The barrel (right) button is down: `POINTER_FLAG_SECONDBUTTON` (drivers that report buttons
    /// raw) or `PEN_FLAG_BARREL` (the barrel held while touching or hovering).
    pub barrel: bool,
    /// The eraser end is in use (`PEN_FLAG_ERASER` or `PEN_FLAG_INVERTED`).
    pub eraser: bool,
    /// Tip pressure 0..1 (`POINTER_PEN_INFO.pressure` is 0..1024). A raw 0 while touching is a
    /// reading below the digitizer's activation threshold; [`advance`] holds the contact's last
    /// non-zero pressure for it.
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// Barrel rotation, degrees 0..360 (`POINTER_PEN_INFO.rotation`).
    pub rotation: f32,
}

/// The button of the pen's synthesized mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// The pen monitor's memory across frames.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    /// The button the synthesized mouse holds, if any.
    held: Option<Button>,
    /// The button the last contact used: a `WM_CONTEXTMENU` right after a contact that used the
    /// right button is the system's echo of that click ([`gesture`]).
    last: Option<Button>,
    /// The last non-zero pressure of the current contact: a digitizer touching lightly reports
    /// raw pressure 0 on many frames (below its activation threshold, not "no pressure"), and
    /// the stroke must hold its last real pressure there. Cleared when the contact ends.
    pressure: f32,
}

/// The mouse messages one frame synthesizes, in the order they are posted, at the frame's
/// position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mouse {
    /// The pen hovers: a plain move.
    Hover,
    /// The contact continues: a move under `button`.
    Move(Button),
    /// The contact starts: a move, then `button` down.
    Press(Button),
    /// The contact ends: a move, then `button` up.
    Release(Button),
}

/// What one frame does: the mouse messages to synthesize and what the UI is told.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub mouse: Mouse,
    pub signal: Signal,
}

/// Advance `state` by one frame and say what to synthesize and report. Only touching frames carry
/// a sample (pressure and the eraser end mean nothing while hovering, and a hover with the eraser
/// end in range must not switch tools), but a cancelled frame is the gesture's retracted contact:
/// [`Signal::Cancel`] tells the UI to suppress the pointer events it already got.
///
/// The button is chosen when the contact starts: a barrel pressed *before* the tap (the right
/// click) is down on the frame that starts it, and pressing the barrel mid-stroke keeps the
/// stroke's button (Photoshop). Every contact ends with a release — a lift, a cancel or the pen
/// leaving the surface — so no button can get stuck.
pub fn advance(state: &mut State, raw: &RawFrame) -> Step {
    if raw.cancelled || !raw.in_contact {
        let ended = state.held.take();
        if ended.is_some() {
            state.last = ended;
        }
        state.pressure = 0.0;
        let mouse = match ended {
            Some(b) => Mouse::Release(b),
            None => Mouse::Hover,
        };
        // Only a cancelled *contact* is a gesture for the UI to act on. A pen that leaves hovering
        // range also reports `POINTER_FLAG_CANCELED`, with nothing held: that cancels no gesture
        // and must not suppress an unrelated stroke (the mouse could be mid-drag).
        let signal = if raw.cancelled && ended.is_some() { Signal::Cancel } else { Signal::Sample(None) };
        return Step { mouse, signal };
    }
    // A raw pressure of 0 while in contact is a reading below the digitizer's activation
    // threshold, not a light touch: report the contact's last non-zero pressure instead, so a
    // light stroke keeps its weight through the zero frames. Before the contact's first non-zero
    // reading there is nothing to hold: that first frame paints at 0, what the pen reported.
    if raw.pressure > 0.0 {
        state.pressure = raw.pressure;
    }
    // The pressure travels as read (the Wacom driver's Tip Feel curve is applied in `windows.rs`).
    let signal = Signal::Sample(Some(Sample { pressure: state.pressure, tilt_x: raw.tilt_x, tilt_y: raw.tilt_y, rotation: raw.rotation, eraser: raw.eraser }));
    match state.held {
        Some(b) => Step { mouse: Mouse::Move(b), signal },
        None => {
            state.last = None;
            let b = if raw.barrel { Button::Right } else { Button::Left };
            state.held = Some(b);
            Step { mouse: Mouse::Press(b), signal }
        }
    }
}

/// A `WM_CONTEXTMENU` right-click gesture's effect (`None` = the message is the system's echo of a
/// right click the frames already synthesized, to be swallowed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gesture {
    /// The button the gesture's tip contact still holds (the system did not cancel it): release it
    /// before the click. A left one committed no stroke: its pointer events must be dropped.
    pub release: Option<Button>,
}

/// Windows Ink's right click (press-and-hold, right tap) arrived as `WM_CONTEXTMENU`; `state` is
/// the monitor's current state. Unless the frames already synthesized that click from the barrel,
/// the gesture itself must become a right click at the pointer — after releasing a tip contact
/// that is still held.
pub fn gesture(state: &mut State) -> Option<Gesture> {
    if state.held == Some(Button::Right) || state.last == Some(Button::Right) {
        return None;
    }
    let release = state.held.take();
    // The click this gesture is about to synthesize echoes back as another `WM_CONTEXTMENU`
    // (`DefWindowProc` turns a right-button release into one): remembering the right button
    // makes the echo take the early return above instead of re-firing the gesture forever. A
    // later contact clears it, so a second hold still fires its own gesture.
    state.last = Some(Button::Right);
    Some(Gesture { release })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(pressure: f32) -> RawFrame {
        RawFrame { in_contact: true, pressure, ..Default::default() }
    }

    fn sample(step: Step) -> Sample {
        match step.signal {
            Signal::Sample(Some(s)) => s,
            other => panic!("expected a sample, got {other:?}"),
        }
    }

    #[test]
    fn a_tip_contact_is_a_move_and_left_click_that_releases_on_the_lift() {
        let mut state = State::default();
        let down = advance(&mut state, &touch(0.5));
        assert_eq!(down.mouse, Mouse::Press(Button::Left), "the tip goes down as the left button");
        assert_eq!(sample(down).pressure, 0.5, "the sample carries the pressure as read");
        assert_eq!(advance(&mut state, &touch(0.8)).mouse, Mouse::Move(Button::Left));
        let up = advance(&mut state, &RawFrame::default());
        assert_eq!(up.mouse, Mouse::Release(Button::Left), "the lift ends the button");
        assert_eq!(up.signal, Signal::Sample(None));
        // A hover now is a plain move.
        assert_eq!(advance(&mut state, &RawFrame::default()).mouse, Mouse::Hover);
    }

    #[test]
    fn hovering_reports_no_sample_so_it_cannot_flip_the_tool() {
        let mut state = State::default();
        let hover = advance(&mut state, &RawFrame { eraser: true, ..Default::default() });
        assert_eq!(hover.signal, Signal::Sample(None), "a hover must not flip the tool");
        assert_eq!(hover.mouse, Mouse::Hover);
    }

    #[test]
    fn the_barrel_button_is_the_right_button_of_the_contact() {
        let mut state = State::default();
        // Pressed before the tap: the frame that starts the contact already carries it.
        let down = advance(&mut state, &RawFrame { in_contact: true, barrel: true, pressure: 0.8, ..Default::default() });
        assert_eq!(down.mouse, Mouse::Press(Button::Right), "barrel + tap is a real right click");
        assert_eq!(sample(down).pressure, 0.8, "the pressure reaches the sample unchanged");
        let drag = RawFrame { in_contact: true, barrel: true, ..Default::default() };
        assert_eq!(advance(&mut state, &drag).mouse, Mouse::Move(Button::Right), "a barrel drag keeps it down");
        assert_eq!(advance(&mut state, &RawFrame::default()).mouse, Mouse::Release(Button::Right));
    }

    #[test]
    fn the_barrel_mid_stroke_keeps_the_stroke_button() {
        let mut state = State::default();
        assert_eq!(advance(&mut state, &touch(0.3)).mouse, Mouse::Press(Button::Left));
        let mid = RawFrame { in_contact: true, barrel: true, ..Default::default() };
        assert_eq!(advance(&mut state, &mid).mouse, Mouse::Move(Button::Left), "the stroke keeps its button (Photoshop)");
        assert_eq!(advance(&mut state, &RawFrame::default()).mouse, Mouse::Release(Button::Left));
    }

    /// A digitizer touching lightly reports raw pressure 0 on many frames (below its activation
    /// threshold): the stroke must hold its last real pressure there, like winit's touch path did
    /// (`normalize_pointer_pressure` reports no force for 0, and the UI kept the previous one) —
    /// otherwise light strokes flicker to nothing and feel weak.
    #[test]
    fn a_zero_pressure_frame_holds_the_contacts_last_pressure() {
        let mut state = State::default();
        advance(&mut state, &touch(0.3));
        let zero = RawFrame { in_contact: true, pressure: 0.0, ..Default::default() };
        let step = advance(&mut state, &zero);
        assert_eq!(sample(step).pressure, 0.3, "the zero frame keeps the last pressure");
        // A real pressure reading replaces the held one.
        assert_eq!(sample(advance(&mut state, &touch(0.1))).pressure, 0.1);
        assert_eq!(sample(advance(&mut state, &zero)).pressure, 0.1);
        // Before the contact's first non-zero reading there is nothing to hold: paint at 0
        // (what the pen reported), never a full-pressure blob.
        let mut state = State::default();
        assert_eq!(sample(advance(&mut state, &zero)).pressure, 0.0);
        // The hold does not cross contacts: the next contact starts from its own first reading.
        advance(&mut state, &touch(0.9));
        advance(&mut state, &RawFrame::default());
        assert_eq!(sample(advance(&mut state, &zero)).pressure, 0.0, "the lift cleared the held pressure");
    }

    #[test]
    fn the_eraser_end_flags_reach_the_sample() {
        let mut state = State::default();
        let raw = RawFrame { in_contact: true, eraser: true, pressure: 0.3, tilt_x: 30.0, tilt_y: -10.0, rotation: 90.0, ..Default::default() };
        let s = sample(advance(&mut state, &raw));
        assert!(s.eraser && s.tilt_x == 30.0 && s.tilt_y == -10.0 && s.rotation == 90.0);
    }

    #[test]
    fn a_cancelled_contact_releases_and_cancels_instead_of_painting() {
        let mut state = State::default();
        advance(&mut state, &touch(0.4));
        let cancelled = RawFrame { cancelled: true, pressure: 0.4, ..Default::default() };
        let step = advance(&mut state, &cancelled);
        assert_eq!(step.mouse, Mouse::Release(Button::Left), "the gesture's contact still gets its up");
        assert_eq!(step.signal, Signal::Cancel, "and the UI drops the stroke it started");
        // Nothing is held afterwards.
        assert_eq!(advance(&mut state, &RawFrame::default()).mouse, Mouse::Hover);
    }

    #[test]
    fn a_cancel_without_a_contact_cancels_nothing() {
        let mut state = State::default();
        let step = advance(&mut state, &RawFrame { cancelled: true, ..Default::default() });
        assert_eq!(step.mouse, Mouse::Hover, "nothing to release");
        assert_eq!(step.signal, Signal::Sample(None), "a pen leaving hover range is not a gesture");
    }

    #[test]
    fn a_gesture_after_a_left_contact_is_the_right_click() {
        let mut state = State::default();
        advance(&mut state, &touch(0.5));
        advance(&mut state, &RawFrame::default());
        assert_eq!(gesture(&mut state), Some(Gesture { release: None }), "the hold's right click");
    }

    /// The click a gesture synthesizes makes `DefWindowProc` post another `WM_CONTEXTMENU` (its
    /// echo): that echo must be swallowed, not re-fire the gesture forever.
    #[test]
    fn a_gestures_own_click_echo_is_swallowed() {
        let mut state = State::default();
        advance(&mut state, &touch(0.5));
        advance(&mut state, &RawFrame::default());
        assert_eq!(gesture(&mut state), Some(Gesture { release: None }));
        assert_eq!(gesture(&mut state), None, "the echo");
        assert_eq!(gesture(&mut state), None, "and so is the echo's echo");
        // A later contact clears it, so a second hold still fires its own gesture.
        advance(&mut state, &touch(0.5));
        advance(&mut state, &RawFrame::default());
        assert_eq!(gesture(&mut state), Some(Gesture { release: None }), "the second hold");
        assert_eq!(gesture(&mut state), None, "whose echo is swallowed too");
    }

    #[test]
    fn a_gesture_while_the_tip_is_still_held_releases_it_first() {
        let mut state = State::default();
        advance(&mut state, &touch(0.5));
        assert_eq!(gesture(&mut state), Some(Gesture { release: Some(Button::Left) }), "the stuck hold is let go");
        assert_eq!(advance(&mut state, &RawFrame::default()).mouse, Mouse::Hover, "and nothing is left down");
    }

    #[test]
    fn a_gesture_after_a_barrel_click_is_the_systems_echo() {
        let mut state = State::default();
        advance(&mut state, &RawFrame { in_contact: true, barrel: true, ..Default::default() });
        advance(&mut state, &RawFrame::default());
        assert_eq!(gesture(&mut state), None, "the frames already clicked: swallow the echo");
    }

    #[test]
    fn a_gesture_while_the_barrel_contact_is_still_down_is_the_echo() {
        let mut state = State::default();
        advance(&mut state, &RawFrame { in_contact: true, barrel: true, ..Default::default() });
        assert_eq!(gesture(&mut state), None);
        assert_eq!(advance(&mut state, &RawFrame::default()).mouse, Mouse::Release(Button::Right), "its release still comes");
    }
}
