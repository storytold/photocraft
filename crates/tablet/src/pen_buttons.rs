//! The pen's barrel (side) button, from the per-event flags a platform reports: whether it is
//! held, whether it went down since the UI last looked, and a click with it (pressed and released
//! within [`CLICK_MS`]). Tablet drivers send that click for a side switch set to Right Click: with
//! Windows Ink, Wacom's driver reports it as a short contact with the secondary button, and the
//! window gets no press of its own for it. Pure, so it is tested on every platform;
//! [`crate::windows`] feeds it. (The click timing follows storytold/photocraft#7616.)

/// Longest press of the button that still counts as a click, in milliseconds. A longer hold is
/// the button held for something else (a right-drag), not a click.
pub const CLICK_MS: u32 = 600;

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
    /// Windows' message time).
    pub fn update(&mut self, barrel: bool, time_ms: u32) {
        match (self.down_at, barrel) {
            (None, true) => {
                self.down_at = Some(time_ms);
                self.pressed = true;
            }
            (Some(at), false) => {
                self.down_at = None;
                self.clicked |= time_ms.wrapping_sub(at) <= CLICK_MS;
            }
            _ => {}
        }
    }

    /// One pen event's contact: `in_contact` while the pen is down, `tip` when its tip presses
    /// (pressure above zero). A contact the tip never presses is the driver's Right Click in the air.
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

    #[test]
    fn leaving_ends_the_contact() {
        let mut t = Tracker::default();
        t.contact(true, false);
        t.leave();
        assert!(t.take().tipless, "reported once");
        assert!(!t.take().tipless);
    }
}
