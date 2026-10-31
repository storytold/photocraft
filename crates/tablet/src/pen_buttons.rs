//! The pen's barrel (side) button, from the per-event flags a platform reports: whether it is
//! held, whether it went down since the UI last looked, and a click with it (pressed and released
//! within [`CLICK_MS`]). Tablet drivers send that click for a side switch set to Right Click: with
//! Windows Ink, Wacom's driver reports it as a short contact with the secondary button, and the
//! window gets no press of its own for it. Pure, so it is tested on every platform;
//! [`crate::windows`] feeds it. (The click timing follows storytold/photocraft#7616.)

/// Longest press of the button that still counts as a click, in milliseconds. A longer hold is
/// the button held for something else (a right-drag), not a click.
pub const CLICK_MS: u32 = 600;

/// Windows' `PEN_MASK_PRESSURE`: the pen info's `pressure` is valid. Without it the device
/// reports no pressure and `pressure` is zero whatever the tip does.
pub const PEN_MASK_PRESSURE: u32 = 0x0000_0001;

/// Whether a pen event's tip presses, for [`Tracker::contact`]: in contact with positive pressure,
/// or in contact at all when the pen reports no pressure (`pen_mask` without
/// [`PEN_MASK_PRESSURE`]). Only a valid zero pressure is a contact the tip doesn't press, so a pen
/// without pressure still right-drags.
pub fn tip_pressed(in_contact: bool, pen_mask: u32, pressure: u32) -> bool {
    in_contact && (pen_mask & PEN_MASK_PRESSURE == 0 || pressure > 0)
}

/// What the barrel button did since the UI last asked ([`Tracker::take`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    /// The button is held now.
    pub held: bool,
    /// It went down since the last [`Tracker::take`] (it may be up again already).
    pub pressed: bool,
    /// It was clicked since the last [`Tracker::take`]: pressed and released within [`CLICK_MS`].
    pub clicked: bool,
    /// The pen's contact is (or, since the last [`Tracker::take`], was) one the tip never pressed:
    /// the driver's Right Click with the pen in the air, not a tap. Its moves are no drag.
    pub tipless: bool,
}

/// Tracks the barrel button across pen events.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tracker {
    /// When the button went down (the event's time in milliseconds), while it is held.
    down_at: Option<u32>,
    pressed: bool,
    clicked: bool,
    /// While the pen is in contact: whether the tip has not pressed yet.
    contact: Option<bool>,
    tipless: bool,
}

impl Tracker {
    /// Nothing held (usable in a `static`).
    pub const DEFAULT: Tracker = Tracker { down_at: None, pressed: false, clicked: false, contact: None, tipless: false };

    /// One pen event: the button's state at `time_ms` (a millisecond clock that may wrap, like
    /// Windows' message time). Returns true when it recorded a press or a click: the UI has news to
    /// take, even if the window got no input of its own for it (a driver's Right Click in the air).
    pub fn update(&mut self, barrel: bool, time_ms: u32) -> bool {
        match (self.down_at, barrel) {
            (None, true) => {
                self.down_at = Some(time_ms);
                self.pressed = true;
                true
            }
            (Some(at), false) => {
                self.down_at = None;
                let click = time_ms.wrapping_sub(at) <= CLICK_MS;
                self.clicked |= click;
                click
            }
            _ => false,
        }
    }

    /// One pen event's contact: `in_contact` while the pen is down, `tip` when its tip presses
    /// ([`tip_pressed`]). A contact the tip never presses is the driver's Right Click in the air.
    pub fn contact(&mut self, in_contact: bool, tip: bool) {
        self.contact = match (self.contact, in_contact) {
            (None, true) => {
                self.tipless |= !tip;
                Some(!tip)
            }
            (Some(tipless), true) => Some(tipless && !tip),
            (_, false) => None,
        };
        if self.contact == Some(false) {
            self.tipless = false;
        }
    }

    /// The pen left the window or its range: the button can't still be held (no click).
    pub fn leave(&mut self) {
        self.down_at = None;
        self.contact = None;
    }

    /// The button is held.
    pub fn held(&self) -> bool {
        self.down_at.is_some()
    }

    /// What happened since the last call; forgets the press and the click.
    pub fn take(&mut self) -> Buttons {
        let tipless = std::mem::replace(&mut self.tipless, self.contact == Some(true));
        Buttons { held: self.held(), pressed: std::mem::take(&mut self.pressed), clicked: std::mem::take(&mut self.clicked), tipless }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_press_and_release_is_a_click() {
        let mut t = Tracker::default();
        t.update(false, 0);
        assert_eq!(t.take(), Buttons::default(), "nothing yet");
        t.update(true, 1000);
        assert_eq!(t.take(), Buttons { held: true, pressed: true, clicked: false, tipless: false });
        t.update(true, 1050);
        assert_eq!(t.take(), Buttons { held: true, pressed: false, clicked: false, tipless: false }, "the press is reported once");
        t.update(false, 1120);
        assert_eq!(t.take(), Buttons { held: false, pressed: false, clicked: true, tipless: false });
        assert_eq!(t.take(), Buttons::default(), "the click is reported once");
    }

    #[test]
    fn a_press_or_a_click_is_news_for_the_ui() {
        // What the Windows hook wakes the UI for: a driver's Right Click in the air may come with
        // no window input of its own, so nothing else would draw the frame that takes it.
        let mut t = Tracker::default();
        assert!(!t.update(false, 0), "nothing happened");
        assert!(t.update(true, 10), "a press");
        assert!(!t.update(true, 20), "still held");
        assert!(t.update(false, 60), "a click");
        assert!(t.update(true, 1000));
        assert!(!t.update(false, 1000 + CLICK_MS + 1), "a long hold ends with no click");
    }

    #[test]
    fn a_click_between_two_takes_reports_both_the_press_and_the_click() {
        let mut t = Tracker::default();
        t.update(true, 10);
        t.update(false, 40);
        assert_eq!(t.take(), Buttons { held: false, pressed: true, clicked: true, tipless: false });
    }

    #[test]
    fn a_long_hold_is_no_click() {
        let mut t = Tracker::default();
        t.update(true, 0);
        t.update(false, CLICK_MS + 1);
        assert_eq!(t.take(), Buttons { held: false, pressed: true, clicked: false, tipless: false });
    }

    #[test]
    fn the_clock_may_wrap() {
        let mut t = Tracker::default();
        t.update(true, u32::MAX - 10);
        t.update(false, 50);
        assert!(t.take().clicked);
    }

    #[test]
    fn leaving_releases_the_button_without_a_click() {
        let mut t = Tracker::default();
        t.update(true, 0);
        t.leave();
        assert_eq!(t.take(), Buttons { held: false, pressed: true, clicked: false, tipless: false });
        t.update(false, 10);
        assert_eq!(t.take(), Buttons::default());
    }

    #[test]
    fn a_contact_the_tip_never_presses_is_tipless_until_it_ends() {
        let mut t = Tracker::default();
        t.contact(true, false);
        assert!(t.take().tipless);
        t.contact(true, false);
        assert!(t.take().tipless, "still the same contact");
        t.contact(false, false);
        assert!(t.take().tipless, "the take after it ends still sees it");
        assert!(!t.take().tipless, "over");
    }

    #[test]
    fn a_short_tipless_contact_is_reported_once_after_it_ends() {
        let mut t = Tracker::default();
        t.contact(true, false);
        t.contact(false, false);
        assert!(t.take().tipless);
        assert!(!t.take().tipless);
    }

    #[test]
    fn a_tip_press_makes_the_contact_a_tap() {
        let mut t = Tracker::default();
        t.contact(true, true);
        assert!(!t.take().tipless);
        let mut t = Tracker::default();
        t.contact(true, false);
        t.contact(true, true);
        assert!(!t.take().tipless, "the tip pressed after all");
        t.contact(true, false);
        assert!(!t.take().tipless, "and it stays a tap");
    }

    /// One contact of `events` pen events with this mask and pressure, as the Windows hook feeds
    /// it, and what the take after it reports.
    fn contact_of(events: usize, pen_mask: u32, pressure: u32) -> Buttons {
        let mut t = Tracker::default();
        for i in 0..events {
            t.update(true, i as u32 * 8);
            t.contact(true, tip_pressed(true, pen_mask, pressure));
        }
        t.take()
    }

    #[test]
    fn a_pen_without_pressure_in_contact_presses_its_tip() {
        // No PEN_MASK_PRESSURE: `pressure` is zero because the device doesn't report it. A real
        // right-drag (the many moves past 12 pt that `pen_barrel` holds back for a tipless
        // contact) must stay one, so the contact is a tap, never tipless.
        assert!(tip_pressed(true, 0, 0));
        assert!(!contact_of(40, 0, 0).tipless, "a right-drag, not the driver's click in the air");
        // Other mask bits (rotation, tilt) without pressure change nothing.
        assert!(!contact_of(40, 0b1110, 0).tipless);
    }

    #[test]
    fn a_valid_zero_pressure_contact_is_tipless() {
        // The driver's Right Click in the air: in contact, pressure reported and zero.
        assert!(!tip_pressed(true, PEN_MASK_PRESSURE, 0));
        assert!(contact_of(40, PEN_MASK_PRESSURE, 0).tipless);
    }

    #[test]
    fn positive_pressure_in_contact_presses_the_tip() {
        assert!(tip_pressed(true, PEN_MASK_PRESSURE, 1));
        assert!(!contact_of(40, PEN_MASK_PRESSURE, 512).tipless);
    }

    #[test]
    fn out_of_contact_the_tip_never_presses() {
        for (mask, pressure) in [(0, 0), (PEN_MASK_PRESSURE, 0), (PEN_MASK_PRESSURE, 512)] {
            assert!(!tip_pressed(false, mask, pressure), "mask {mask}, pressure {pressure}");
        }
    }

    #[test]
    fn leaving_ends_the_contact() {
        let mut t = Tracker::default();
        t.contact(true, false);
        t.leave();
        assert!(t.take().tipless, "reported once");
        assert!(!t.take().tipless);
    }
}
