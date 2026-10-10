//! PhotoCraft's local Windows pen mapping. No FFI: use the fields already read by winit.
//! Hovering barrel buttons live in PEN_FLAG_BARREL, not POINTER_FLAG_SECONDBUTTON.
use super::DEVICE_ID;
use crate::dpi::PhysicalPosition;
use crate::event::{ElementState, Force, MouseButton, Touch, TouchPhase, WindowEvent};
use windows_sys::Win32::UI::Input::Pointer::{POINTER_FLAG_CANCELED, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_SECONDBUTTON};
use windows_sys::Win32::UI::WindowsAndMessaging::PEN_FLAG_BARREL;

#[derive(Default)]
pub(crate) struct PenInput {
    tip: bool,
    contact: bool,
    barrel: bool,
    id: Option<u32>,
    position: PhysicalPosition<f64>,
}

impl PenInput {
    pub(crate) fn leave(&mut self, id: u32) -> Vec<WindowEvent> {
        if self.id == Some(id) {
            self.update(id, 0, 0, self.position, None)
        } else {
            Vec::new()
        }
    }

    pub(crate) fn update(&mut self, id: u32, flags: u32, pen_flags: u32, position: PhysicalPosition<f64>, force: Option<Force>) -> Vec<WindowEvent> {
        let live = flags & POINTER_FLAG_INRANGE != 0 && flags & POINTER_FLAG_CANCELED == 0;
        let contact = live && flags & POINTER_FLAG_INCONTACT != 0;
        let barrel = live && (pen_flags & PEN_FLAG_BARREL != 0 || flags & POINTER_FLAG_SECONDBUTTON != 0);
        let tip = contact && !barrel;
        let mut events = Vec::new();
        // Keep Touch force for egui's stylus reader. Only tip contact may emit Started,
        // because egui-winit turns Started into a left press. Barrel contact emits Move.
        let phase = if tip && !self.tip {
            Some(TouchPhase::Started)
        } else if (self.tip && !tip) || (self.contact && !contact) {
            Some(TouchPhase::Ended)
        } else if contact {
            Some(TouchPhase::Moved)
        } else {
            None
        };
        let touch = |phase| WindowEvent::Touch(Touch { device_id: DEVICE_ID, phase, location: position, force, id: u64::from(id) });
        if let Some(phase) = phase {
            events.push(touch(phase));
            if phase == TouchPhase::Ended && contact {
                events.push(touch(TouchPhase::Moved));
            }
        }
        // Touch End clears egui's pointer. Restore its position before a barrel button
        // event, and retain hover after lifting the tip (a pen is still in range).
        // Started and Moved already position egui's pointer through its Touch adapter.
        // Avoid duplicating stroke points; only hover or a final End needs this.
        if phase.is_none() || (phase == Some(TouchPhase::Ended) && !contact) {
            events.push(WindowEvent::CursorMoved { device_id: DEVICE_ID, position });
        }
        if barrel != self.barrel {
            events.push(WindowEvent::MouseInput {
                device_id: DEVICE_ID,
                state: if barrel { ElementState::Pressed } else { ElementState::Released },
                button: MouseButton::Right,
            });
        }
        if !live {
            events.push(WindowEvent::CursorLeft { device_id: DEVICE_ID });
        }
        self.tip = tip;
        self.contact = contact;
        self.barrel = barrel;
        self.id = Some(id);
        self.position = position;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const HOVER: u32 = POINTER_FLAG_INRANGE;
    const CONTACT: u32 = HOVER | POINTER_FLAG_INCONTACT;
    fn update(pen: &mut PenInput, flags: u32, barrel: bool) -> Vec<WindowEvent> {
        pen.update(1, flags, if barrel { PEN_FLAG_BARREL } else { 0 }, PhysicalPosition::new(100.0, 200.0), Some(Force::Normalized(0.7)))
    }
    fn right(events: &[WindowEvent], state: ElementState) -> bool {
        events.iter().any(|e| matches!(e, WindowEvent::MouseInput { button: MouseButton::Right, state: s, .. } if *s == state))
    }
    fn phase(events: &[WindowEvent], want: TouchPhase) -> bool {
        events.iter().any(|e| matches!(e, WindowEvent::Touch(t) if t.phase == want))
    }
    #[test]
    fn hovering_barrel_emits_one_right_press_and_release_without_left_contact() {
        let mut pen = PenInput::default();
        assert!(!right(&update(&mut pen, HOVER, false), ElementState::Pressed));
        let down = update(&mut pen, HOVER, true);
        assert!(right(&down, ElementState::Pressed));
        assert!(!phase(&down, TouchPhase::Started));
        assert!(!right(&update(&mut pen, HOVER, true), ElementState::Pressed));
        assert!(right(&update(&mut pen, HOVER, false), ElementState::Released));
    }
    #[test]
    fn barrel_contact_preserves_pressure_without_painting_a_left_stroke() {
        let mut pen = PenInput::default();
        let events = update(&mut pen, CONTACT | POINTER_FLAG_SECONDBUTTON, true);
        assert!(right(&events, ElementState::Pressed));
        assert!(!phase(&events, TouchPhase::Started));
        assert!(events.iter().any(|e| matches!(e, WindowEvent::Touch(Touch { force: Some(Force::Normalized(f)), .. }) if *f == 0.7)));
        let lift = update(&mut pen, HOVER, true);
        assert!(phase(&lift, TouchPhase::Ended));
        assert!(!right(&lift, ElementState::Released), "lifting the tip does not release a held barrel");
        assert!(right(&update(&mut pen, HOVER, false), ElementState::Released));
    }
    #[test]
    fn normal_tip_stroke_retains_pressure_and_hover_after_release() {
        let mut pen = PenInput::default();
        assert!(phase(&update(&mut pen, CONTACT, false), TouchPhase::Started));
        assert!(phase(&update(&mut pen, CONTACT, false), TouchPhase::Moved));
        let lift = update(&mut pen, HOVER, false);
        assert!(phase(&lift, TouchPhase::Ended));
        assert!(matches!(lift.last(), Some(WindowEvent::CursorMoved { .. })));
        assert!(!right(&lift, ElementState::Pressed));
    }
    #[test]
    fn leaving_range_or_canceling_releases_a_held_button() {
        for flags in [0, HOVER | POINTER_FLAG_CANCELED] {
            let mut pen = PenInput::default();
            update(&mut pen, HOVER, true);
            let end = update(&mut pen, flags, true);
            assert!(right(&end, ElementState::Released));
            assert!(matches!(end.last(), Some(WindowEvent::CursorLeft { .. })));
        }
    }
    #[test]
    fn pressing_barrel_during_tip_contact_ends_left_before_right_press() {
        let mut pen = PenInput::default();
        update(&mut pen, CONTACT, false);
        let events = update(&mut pen, CONTACT | POINTER_FLAG_SECONDBUTTON, true);
        assert!(matches!(events.first(), Some(WindowEvent::Touch(Touch { phase: TouchPhase::Ended, .. }))));
        assert!(right(&events, ElementState::Pressed));
        assert!(!phase(&events, TouchPhase::Started));
    }
    #[test]
    fn leave_only_releases_the_tracked_pen() {
        let mut pen = PenInput::default();
        update(&mut pen, HOVER, true);
        assert!(pen.leave(2).is_empty());
        assert!(right(&pen.leave(1), ElementState::Released));
    }
}
