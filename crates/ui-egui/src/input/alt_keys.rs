//! Alt as its own key says it is (#10544).
//!
//! Every ⌥-click on the canvas (the Brush's Eyedropper, the Clone Stamp's source point, the Mixer
//! Brush's paint load) reads the Alt in the *modifier state* the windowing layer attaches to the
//! pointer events. On Windows that state can miss a held Alt: the mouse button message resyncs it
//! from the keyboard state the message was queued with, and a click that arrives from a precision
//! touchpad can carry a keyboard state older than the Alt key press that came before it. The click
//! then reports no Alt although the on-screen keyboard (or the physical key) shows it down, so the
//! Brush paints, the Clone Stamp paints from a stale source, and every now and then a click gets it
//! right (#8049 saw the same state on the wheel).
//!
//! The Alt key's own press and release always arrive, in order with the pointer events, so this
//! folds them into the modifiers of the events after them: from an Alt key press to its release,
//! pointer buttons, wheel notches and keys carry `alt`, and the modifier state egui keeps for the
//! frame (`ModifiersChanged`) follows the key. It runs on the frame's raw input, before anything
//! reads it, so every consumer (the canvas tools, the shortcuts, the panels) sees the same Alt.
//!
//! A lost key release cannot leave Alt stuck across a focus change: egui forgets its held keys
//! when the window loses focus, and so does this.

use egui::{Context, Event, Key, Modifiers, RawInput};

/// Which Alt key an event names: 0 for the left one, 1 for the right one (AltGr).
fn alt_slot(key: Key) -> Option<usize> {
    match key {
        Key::AltLeft => Some(0),
        Key::AltRight => Some(1),
        _ => None,
    }
}

/// The Alt keys held and the modifier state in effect at some point of the event stream.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AltKeys {
    /// The left and right Alt keys, down from their press event to their release event.
    held: [bool; 2],
    /// The modifier state egui will have after the events seen so far.
    modifiers: Modifiers,
}

impl AltKeys {
    /// The state at the end of the previous frame: egui's held keys and modifiers.
    pub(crate) fn of(ctx: &Context) -> Self {
        ctx.input(|i| Self { held: [i.key_down(Key::AltLeft), i.key_down(Key::AltRight)], modifiers: i.modifiers })
    }

    /// Is an Alt key down?
    fn down(&self) -> bool {
        self.held.iter().any(|h| *h)
    }

    /// `modifiers` with Alt added while an Alt key is down.
    fn fixed(&self, modifiers: Modifiers) -> Modifiers {
        Modifiers { alt: modifiers.alt || self.down(), ..modifiers }
    }

    /// Fold one frame's events: `events` come out with Alt where the Alt key says it was held.
    pub(crate) fn fold(mut self, events: &mut Vec<Event>) {
        if !events.iter().any(|e| matches!(e, Event::Key { .. } | Event::ModifiersChanged(_) | Event::WindowFocused(false)) || carries_modifiers(e)) {
            return;
        }
        let mut out = Vec::with_capacity(events.len() + 2);
        for event in events.drain(..) {
            match event {
                Event::Key { key, pressed, repeat, physical_key, modifiers } => {
                    let slot = alt_slot(key);
                    if let Some(slot) = slot
                        && let Some(h) = self.held.get_mut(slot)
                    {
                        *h = pressed;
                    }
                    // The Alt key's own event keeps what the platform stamped on it.
                    let modifiers = if slot.is_some() { modifiers } else { self.fixed(modifiers) };
                    out.push(Event::Key { key, pressed, repeat, physical_key, modifiers });
                    if slot.is_some() && self.down() != self.modifiers.alt {
                        // The platform's modifier state disagrees with the key (it missed the
                        // press, or never reported the release): the key is right.
                        self.modifiers.alt = self.down();
                        out.push(Event::ModifiersChanged(self.modifiers));
                    }
                }
                // The platform's modifiers, with a held Alt kept: a click resyncs them from a
                // keyboard state that may predate the Alt press.
                Event::ModifiersChanged(m) => {
                    self.modifiers = self.fixed(m);
                    out.push(Event::ModifiersChanged(self.modifiers));
                }
                Event::PointerButton { pos, button, pressed, modifiers } => {
                    out.push(Event::PointerButton { pos, button, pressed, modifiers: self.fixed(modifiers) });
                }
                Event::MouseWheel { unit, delta, phase, source, modifiers } => {
                    out.push(Event::MouseWheel { unit, delta, phase, source, modifiers: self.fixed(modifiers) });
                }
                // egui forgets every held key and modifier here, and so do we.
                Event::WindowFocused(false) => {
                    self = Self::default();
                    out.push(Event::WindowFocused(false));
                }
                other => out.push(other),
            }
        }
        *events = out;
    }
}

/// Does `event` carry the modifier state of its moment?
fn carries_modifiers(event: &Event) -> bool {
    matches!(event, Event::PointerButton { .. } | Event::MouseWheel { .. })
}

/// Fold the Alt key into `raw`'s events (call from `raw_input_hook`, after every source of events
/// has added its own and before the frame runs).
pub(crate) fn fold_alt(ctx: &Context, raw: &mut RawInput) {
    AltKeys::of(ctx).fold(&mut raw.events);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{PointerButton, pos2};

    fn alt_key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
    }

    fn click(pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos: pos2(5.0, 5.0), button: PointerButton::Primary, pressed, modifiers }
    }

    fn fold(state: AltKeys, mut events: Vec<Event>) -> Vec<Event> {
        state.fold(&mut events);
        events
    }

    fn alts(events: &[Event]) -> Vec<bool> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::PointerButton { modifiers, .. } | Event::MouseWheel { modifiers, .. } | Event::Key { modifiers, .. } => Some(modifiers.alt),
                Event::ModifiersChanged(m) => Some(m.alt),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_click_after_the_alt_press_in_the_same_frame_carries_alt() {
        let events = fold(AltKeys::default(), vec![alt_key(Key::AltLeft, true, Modifiers::NONE), click(true, Modifiers::NONE), click(false, Modifiers::NONE)]);
        // The Alt key, the ModifiersChanged the key implies, then the press and the release.
        assert!(matches!(events[1], Event::ModifiersChanged(m) if m.alt));
        assert_eq!(alts(&events), [false, true, true, true]);
    }

    #[test]
    fn a_click_a_frame_after_the_alt_press_carries_alt() {
        // Frame 1: Alt goes down. Frame 2: a press and a release in one frame (a touchpad tap),
        // whose modifiers the platform took from a keyboard state older than the Alt press.
        let held = AltKeys { held: [true, false], modifiers: Modifiers::ALT };
        let events = fold(held, vec![Event::PointerMoved(pos2(5.0, 5.0)), click(true, Modifiers::NONE), click(false, Modifiers::NONE)]);
        assert_eq!(alts(&events), [true, true]);
    }

    #[test]
    fn a_resync_to_a_stale_keyboard_state_keeps_the_held_alt() {
        // The Alt press, then the mouse button resyncing the modifiers to a state without it.
        let state = AltKeys { held: [true, false], modifiers: Modifiers::ALT };
        let events = fold(state, vec![Event::ModifiersChanged(Modifiers::NONE), click(true, Modifiers::NONE), click(false, Modifiers::NONE)]);
        assert_eq!(alts(&events), [true, true, true]);
    }

    #[test]
    fn the_release_ends_alt_even_when_the_platform_never_reported_it() {
        // The platform never had Alt: the press made it appear, so the release must remove it.
        let events = fold(
            AltKeys::default(),
            vec![
                alt_key(Key::AltRight, true, Modifiers::NONE),
                click(true, Modifiers::NONE),
                alt_key(Key::AltRight, false, Modifiers::NONE),
                click(false, Modifiers::NONE),
            ],
        );
        let last_mods = events.iter().rev().find_map(|e| if let Event::ModifiersChanged(m) = e { Some(*m) } else { None });
        assert_eq!(last_mods, Some(Modifiers::NONE), "egui's modifier state follows the release");
        let button_alts: Vec<bool> =
            events.iter().filter_map(|e| if let Event::PointerButton { modifiers, .. } = e { Some(modifiers.alt) } else { None }).collect();
        assert_eq!(button_alts, [true, false], "the press had Alt, the release (after the key's) does not");
    }

    #[test]
    fn one_alt_key_up_keeps_alt_while_the_other_is_down() {
        let state = AltKeys { held: [true, true], modifiers: Modifiers::ALT };
        let events = fold(state, vec![alt_key(Key::AltLeft, false, Modifiers::ALT), click(true, Modifiers::NONE)]);
        assert_eq!(alts(&events), [true, true], "no ModifiersChanged, and the click still has Alt");
        assert!(!events.iter().any(|e| matches!(e, Event::ModifiersChanged(_))));
    }

    #[test]
    fn nothing_changes_without_an_alt_key() {
        let plain = vec![Event::PointerMoved(pos2(1.0, 1.0)), click(true, Modifiers::SHIFT), click(false, Modifiers::SHIFT)];
        assert_eq!(fold(AltKeys::default(), plain.clone()), plain);
        // Platform-reported Alt with no Alt key event stays: the fold only adds.
        let reported = vec![click(true, Modifiers::ALT), click(false, Modifiers::ALT)];
        assert_eq!(fold(AltKeys::default(), reported.clone()), reported);
    }

    #[test]
    fn losing_the_focus_drops_the_held_alt() {
        let state = AltKeys { held: [true, false], modifiers: Modifiers::ALT };
        let events = fold(state, vec![Event::WindowFocused(false), Event::WindowFocused(true), click(true, Modifiers::NONE)]);
        assert_eq!(alts(&events), [false], "an Alt that was down when the window lost the focus is not carried back");
    }

    #[test]
    fn other_keys_carry_the_held_alt_and_the_alt_key_keeps_its_own_stamp() {
        let state = AltKeys { held: [true, false], modifiers: Modifiers::ALT };
        let events = fold(state, vec![Event::Key { key: Key::S, physical_key: Some(Key::S), pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        assert_eq!(alts(&events), [true]);
        let events = fold(AltKeys::default(), vec![alt_key(Key::AltLeft, true, Modifiers::NONE)]);
        assert!(matches!(events[0], Event::Key { modifiers, .. } if !modifiers.alt), "the key's own stamp is the platform's");
    }

    #[test]
    fn the_context_state_seeds_the_fold() {
        let ctx = Context::default();
        let mut raw = RawInput { events: vec![alt_key(Key::AltLeft, true, Modifiers::ALT)], ..Default::default() };
        fold_alt(&ctx, &mut raw);
        ctx.run_ui(raw, |_| {}).textures_delta.clear();
        assert_eq!(AltKeys::of(&ctx).held, [true, false]);
        // A later frame whose click lacks Alt gets it from the key that is still down.
        let mut raw = RawInput { events: vec![click(true, Modifiers::NONE)], ..Default::default() };
        fold_alt(&ctx, &mut raw);
        assert_eq!(alts(&raw.events), [true]);
    }
}
