//! Android contact arbitration, compiled into the APK's patched winit backend by Nix.
//! Host tests compile this same file. Time and decoded Android events are explicit inputs.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

pub const TOUCH_RESUME_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Finger,
    Palm,
    Mouse,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub device: i32,
    pub pointer: i32,
    pub tool: Tool,
    pub position: [f64; 2],
    pub pressure: f64,
}

// Android may reuse pointer 0 across the pen and touchscreen on the same device.
// A late palm Up must not release a pen stroke with that pointer number.
type Key = (i32, i32, u8);
impl Contact {
    fn key(self) -> Key {
        let class = match self.tool {
            Tool::Pen => 1,
            Tool::Mouse => 2,
            _ => 0,
        };
        (self.device, self.pointer, class)
    }
    fn is_touch(self) -> bool {
        !matches!(self.tool, Tool::Pen | Tool::Mouse)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Down(usize),
    Move,
    Up(usize),
    CancelPointer(usize),
    Hover,
    HoverExit,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    pub contact: Contact,
    // Unlike Android's pointer number, this ID is unique across live pen/touch devices.
    pub id: u64,
    pub phase: Phase,
}

#[derive(Debug, Default)]
pub struct PalmRejection {
    hovering: BTreeSet<Key>,
    pen_down: BTreeSet<Key>,
    active: BTreeMap<Key, Output>,
    blocked: BTreeSet<Key>,
    resume_at: Duration,
    next_id: u64,
}

impl PalmRejection {
    /// For diagnostic logging: pen presence and the number of quarantined contacts.
    pub fn status(&self) -> (bool, usize) {
        (!self.hovering.is_empty() || !self.pen_down.is_empty(), self.blocked.len())
    }

    /// `contacts` is the complete native motion-event snapshot, not just the changed pointer.
    /// Observe every pen before routing fingers, regardless of pointer order within the event.
    pub fn motion(&mut self, now: Duration, device: i32, action: Action, contacts: &[Contact]) -> Vec<Output> {
        match action {
            Action::Down(index) | Action::Up(index) | Action::CancelPointer(index) if contacts.get(index).is_none() => return Vec::new(),
            _ => {}
        }
        let was_near = self.status().0;
        let mut output = Vec::new();
        match action {
            Action::Down(_) | Action::Move => {
                for contact in contacts.iter().filter(|c| c.tool == Tool::Pen) {
                    self.hovering.remove(&contact.key());
                    self.pen_down.insert(contact.key());
                }
            }
            Action::Hover => {
                for contact in contacts.iter().filter(|c| c.tool == Tool::Pen) {
                    self.hovering.insert(contact.key());
                }
            }
            Action::HoverExit => self.hovering.retain(|key| key.0 != device),
            Action::Up(index) | Action::CancelPointer(index) => {
                if let Some(contact) = contacts.get(index) {
                    if contact.tool == Tool::Pen {
                        self.pen_down.remove(&contact.key());
                        self.hovering.remove(&contact.key());
                    }
                    if matches!(action, Action::CancelPointer(_)) {
                        self.cancel(contact.key(), &mut output);
                        self.blocked.remove(&contact.key());
                    }
                }
            }
            Action::Cancel => {
                self.hovering.retain(|key| key.0 != device);
                self.pen_down.retain(|key| key.0 != device);
                let keys: Vec<_> = self.active.keys().copied().filter(|key| key.0 == device).collect();
                for key in keys {
                    self.cancel(key, &mut output);
                }
                self.blocked.retain(|key| key.0 != device);
            }
        }
        if was_near && !self.status().0 {
            self.resume_at = now.saturating_add(TOUCH_RESUME_DELAY);
        }
        let reject_touch = self.status().0 || now < self.resume_at;
        if reject_touch {
            let keys: Vec<_> = self.active.iter().filter(|(_, event)| event.contact.is_touch()).map(|(key, _)| *key).collect();
            for key in keys {
                self.cancel(key, &mut output);
                self.blocked.insert(key);
            }
        }
        match action {
            Action::Down(index) | Action::Up(index) => {
                if let Some(contact) = contacts.get(index) {
                    let phase = if matches!(action, Action::Down(_)) { Phase::Started } else { Phase::Ended };
                    self.route(*contact, phase, reject_touch, &mut output);
                }
            }
            Action::Move => {
                for contact in contacts {
                    self.route(*contact, Phase::Moved, reject_touch, &mut output);
                }
            }
            Action::Hover | Action::HoverExit | Action::Cancel | Action::CancelPointer(_) => {}
        }
        output
    }

    fn cancel(&mut self, key: Key, output: &mut Vec<Output>) {
        if let Some(mut event) = self.active.remove(&key) {
            event.phase = Phase::Cancelled;
            output.push(event);
        }
    }

    fn route(&mut self, mut contact: Contact, phase: Phase, reject_touch: bool, output: &mut Vec<Output>) {
        let key = contact.key();
        if phase == Phase::Started {
            // A new Down starts a new lifetime even if Android reused an old pointer ID.
            self.cancel(key, output);
            self.blocked.remove(&key);
        }
        if contact.tool == Tool::Palm || (contact.is_touch() && reject_touch) {
            self.cancel(key, output);
            self.blocked.insert(key);
        }
        if phase == Phase::Ended {
            self.blocked.remove(&key);
            if let Some(mut event) = self.active.remove(&key) {
                event.phase = Phase::Ended;
                // End at the last accepted location, never at an invalid/rejected sample.
                output.push(event);
            }
            return;
        }
        if self.blocked.contains(&key) {
            return;
        }
        if !contact.position.iter().all(|p| p.is_finite()) {
            self.cancel(key, output);
            self.blocked.insert(key);
            return;
        }
        contact.pressure = if contact.pressure.is_finite() { contact.pressure.clamp(0.0, 1.0) } else { 0.0 };
        let id = if phase == Phase::Started {
            let Some(next) = self.next_id.checked_add(1) else { return };
            self.next_id = next;
            next
        } else if let Some(previous) = self.active.get(&key) {
            previous.id
        } else {
            // Never restart a rejected contact from Move. Require a fresh Down.
            return;
        };
        let event = Output { contact, id, phase };
        self.active.insert(key, event);
        output.push(event);
    }

    /// Focus loss, suspend and native-window destruction end every forwarded contact.
    /// Clear hover as well, because Android need not send HoverExit during these transitions.
    pub fn reset(&mut self) -> Vec<Output> {
        let output = std::mem::take(&mut self.active)
            .into_values()
            .map(|mut e| {
                e.phase = Phase::Cancelled;
                e
            })
            .collect();
        self.hovering.clear();
        self.pen_down.clear();
        self.blocked.clear();
        self.resume_at = Duration::ZERO;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contact(tool: Tool, device: i32, pointer: i32) -> Contact {
        Contact { tool, device, pointer, position: [100.0, 100.0], pressure: 0.7 }
    }
    fn send(filter: &mut PalmRejection, ms: u64, action: Action, contacts: &[Contact]) -> Vec<Output> {
        filter.motion(Duration::from_millis(ms), contacts.first().map_or(0, |c| c.device), action, contacts)
    }

    #[test]
    fn hovering_pen_blocks_touch_without_emitting_a_press() {
        let mut f = PalmRejection::default();
        send(&mut f, 0, Action::Hover, &[contact(Tool::Pen, 2, 0)]);
        assert!(send(&mut f, 1, Action::Down(0), &[contact(Tool::Finger, 1, 0)]).is_empty());
    }

    #[test]
    fn pen_arrival_cancels_existing_touch_before_starting_pen() {
        let mut f = PalmRejection::default();
        let finger = contact(Tool::Finger, 1, 0);
        let pen = contact(Tool::Pen, 2, 0);
        let first = send(&mut f, 0, Action::Down(0), &[finger]);
        let events = send(&mut f, 1, Action::Down(0), &[pen]);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].phase, Phase::Cancelled);
        assert_eq!(events[0].id, first[0].id);
        assert_eq!(events[1].phase, Phase::Started);
        assert_ne!(events[0].id, events[1].id);
    }

    #[test]
    fn rejected_contact_stays_blocked_until_lift_even_after_pen_leaves() {
        let mut f = PalmRejection::default();
        let pen = contact(Tool::Pen, 2, 0);
        let finger = contact(Tool::Finger, 1, 0);
        send(&mut f, 0, Action::Hover, &[pen]);
        send(&mut f, 1, Action::Down(0), &[finger]);
        send(&mut f, 2, Action::HoverExit, &[pen]);
        assert!(send(&mut f, 1000, Action::Move, &[finger]).is_empty());
        assert!(send(&mut f, 1001, Action::Up(0), &[finger]).is_empty());
        assert_eq!(send(&mut f, 1002, Action::Down(0), &[finger]).len(), 1);
    }

    #[test]
    fn touch_and_mouse_work_normally_without_a_pen() {
        for tool in [Tool::Finger, Tool::Unknown, Tool::Mouse] {
            let mut f = PalmRejection::default();
            let c = contact(tool, 1, 0);
            let start = send(&mut f, 0, Action::Down(0), &[c]);
            let moved = send(&mut f, 1, Action::Move, &[c]);
            let end = send(&mut f, 2, Action::Up(0), &[c]);
            assert_eq!(start[0].phase, Phase::Started);
            assert_eq!(moved[0].phase, Phase::Moved);
            assert_eq!(end[0].phase, Phase::Ended);
            assert_eq!(start[0].id, end[0].id);
            assert_eq!(moved[0].contact.pressure, c.pressure);
        }
    }

    #[test]
    fn touch_resumes_at_the_delay_boundary_not_before() {
        let mut f = PalmRejection::default();
        let pen = contact(Tool::Pen, 2, 0);
        let finger = contact(Tool::Finger, 1, 0);
        send(&mut f, 0, Action::Hover, &[pen]);
        send(&mut f, 100, Action::HoverExit, &[pen]);
        assert!(send(&mut f, 349, Action::Down(0), &[finger]).is_empty());
        send(&mut f, 349, Action::Up(0), &[finger]);
        assert_eq!(send(&mut f, 350, Action::Down(0), &[finger]).len(), 1);
    }

    #[test]
    fn palm_up_cannot_release_a_pen_with_the_same_native_id() {
        let mut f = PalmRejection::default();
        let pen = contact(Tool::Pen, 1, 0);
        let finger = contact(Tool::Finger, 1, 0);
        send(&mut f, 0, Action::Down(0), &[finger]);
        let pen_start = send(&mut f, 1, Action::Down(0), &[pen]);
        assert!(send(&mut f, 2, Action::Up(0), &[finger]).is_empty());
        let moved = send(&mut f, 3, Action::Move, &[pen]);
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].id, pen_start[1].id);
        assert_eq!(moved[0].phase, Phase::Moved);
    }

    #[test]
    fn full_snapshot_detects_pen_before_routing_finger_at_lower_index() {
        let mut f = PalmRejection::default();
        let finger = contact(Tool::Finger, 1, 0);
        let pen = contact(Tool::Pen, 1, 1);
        assert!(send(&mut f, 0, Action::Down(0), &[finger, pen]).is_empty());
        let started = send(&mut f, 1, Action::Down(1), &[finger, pen]);
        assert_eq!(started.len(), 1);
        let moved = send(&mut f, 2, Action::Move, &[finger, pen]);
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].contact.tool, Tool::Pen);
    }

    #[test]
    fn explicit_palm_is_rejected_without_any_pen() {
        let mut f = PalmRejection::default();
        let palm = contact(Tool::Palm, 1, 0);
        assert!(send(&mut f, 0, Action::Down(0), &[palm]).is_empty());
        assert!(send(&mut f, 1, Action::Move, &[palm]).is_empty());
        assert!(send(&mut f, 2, Action::Up(0), &[palm]).is_empty());
    }

    #[test]
    fn finger_reclassified_as_palm_gets_cancel_not_release() {
        let mut f = PalmRejection::default();
        let finger = contact(Tool::Finger, 1, 0);
        let palm = Contact { tool: Tool::Palm, ..finger };
        send(&mut f, 0, Action::Down(0), &[finger]);
        let cancelled = send(&mut f, 1, Action::Move, &[palm]);
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].phase, Phase::Cancelled);
        assert!(send(&mut f, 2, Action::Up(0), &[palm]).is_empty());
    }

    #[test]
    fn pen_without_hover_still_protects_drawing_and_delays_touch_after_up() {
        let mut f = PalmRejection::default();
        let pen = contact(Tool::Pen, 2, 0);
        let finger = contact(Tool::Finger, 1, 0);
        send(&mut f, 0, Action::Down(0), &[pen]);
        assert!(send(&mut f, 1, Action::Down(0), &[finger]).is_empty());
        send(&mut f, 2, Action::Up(0), &[finger]);
        assert_eq!(send(&mut f, 10, Action::Up(0), &[pen])[0].phase, Phase::Ended);
        assert!(send(&mut f, 259, Action::Down(0), &[finger]).is_empty());
        send(&mut f, 259, Action::Up(0), &[finger]);
        assert_eq!(send(&mut f, 260, Action::Down(0), &[finger]).len(), 1);
    }

    #[test]
    fn device_cancel_does_not_clear_a_different_hovering_pen() {
        let mut f = PalmRejection::default();
        let pen = contact(Tool::Pen, 2, 0);
        let finger = contact(Tool::Finger, 1, 0);
        send(&mut f, 0, Action::Hover, &[pen]);
        send(&mut f, 1, Action::Down(0), &[finger]);
        send(&mut f, 2, Action::Cancel, &[finger]);
        assert!(f.status().0);
        assert!(send(&mut f, 3, Action::Down(0), &[finger]).is_empty());
        send(&mut f, 4, Action::Cancel, &[pen]);
        assert!(!f.status().0);
    }

    #[test]
    fn individual_cancel_does_not_cancel_another_contact() {
        let mut f = PalmRejection::default();
        let a = contact(Tool::Finger, 1, 0);
        let b = contact(Tool::Finger, 1, 1);
        send(&mut f, 0, Action::Down(0), &[a]);
        let second = send(&mut f, 1, Action::Down(1), &[a, b]);
        let cancelled = send(&mut f, 2, Action::CancelPointer(0), &[a, b]);
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].phase, Phase::Cancelled);
        assert_eq!(send(&mut f, 3, Action::Move, &[b])[0].id, second[0].id);
    }

    #[test]
    fn focus_loss_clears_hover_and_releases_every_active_contact() {
        let mut f = PalmRejection::default();
        let pen = contact(Tool::Pen, 2, 0);
        let mouse = contact(Tool::Mouse, 3, 0);
        send(&mut f, 0, Action::Down(0), &[pen]);
        send(&mut f, 1, Action::Down(0), &[mouse]);
        let cancelled = f.reset();
        assert_eq!(cancelled.len(), 2);
        assert!(cancelled.iter().all(|c| c.phase == Phase::Cancelled));
        assert_eq!(f.status(), (false, 0));
        assert_eq!(send(&mut f, 2, Action::Down(0), &[contact(Tool::Finger, 1, 0)]).len(), 1);
    }

    #[test]
    fn mouse_is_not_rejected_and_pressure_does_not_identify_a_pen() {
        let mut f = PalmRejection::default();
        let pen = Contact { pressure: 0.0, ..contact(Tool::Pen, 2, 0) };
        send(&mut f, 0, Action::Hover, &[pen]);
        assert_eq!(send(&mut f, 1, Action::Down(0), &[contact(Tool::Mouse, 3, 0)]).len(), 1);
        assert!(send(&mut f, 2, Action::Down(0), &[contact(Tool::Unknown, 4, 0)]).is_empty());
        let finger = Contact { pressure: 1.0, ..contact(Tool::Finger, 1, 0) };
        assert!(send(&mut f, 3, Action::Down(0), &[finger]).is_empty());
    }

    #[test]
    fn multiple_pens_require_the_last_one_to_leave() {
        let mut f = PalmRejection::default();
        let a = contact(Tool::Pen, 2, 0);
        let b = contact(Tool::Pen, 3, 0);
        send(&mut f, 0, Action::Hover, &[a]);
        send(&mut f, 1, Action::Hover, &[b]);
        send(&mut f, 2, Action::HoverExit, &[a]);
        assert!(f.status().0);
        send(&mut f, 3, Action::HoverExit, &[b]);
        assert!(!f.status().0);
    }

    #[test]
    fn malformed_input_does_not_start_a_contact_or_leave_pen_latched() {
        let mut f = PalmRejection::default();
        let pen = contact(Tool::Pen, 2, 0);
        assert!(send(&mut f, 0, Action::Down(usize::MAX), &[pen]).is_empty());
        assert!(!f.status().0);
        assert!(send(&mut f, 1, Action::Move, &[contact(Tool::Finger, 1, 0)]).is_empty());
        let invalid = Contact { position: [f64::NAN, 1.0], ..contact(Tool::Finger, 1, 0) };
        assert!(send(&mut f, 2, Action::Down(0), &[invalid]).is_empty());
        let valid = Contact { pressure: f64::INFINITY, ..contact(Tool::Finger, 1, 0) };
        assert_eq!(send(&mut f, 3, Action::Down(0), &[valid])[0].contact.pressure, 0.0);
    }
}
