//! Recover logical secondary clicks from Windows Ink's physical barrel-button service.
//! winit emits primary pointer events for pen contact, even with the barrel button held.

use egui::{Context, Event, PointerButton, Pos2, RawInput};

#[derive(Clone, Debug, Default)]
pub(crate) struct PenButton {
    down: bool,
    suppressed_contact: bool,
    position: Option<Pos2>,
}

impl PenButton {
    pub fn normalize(&mut self, ctx: &Context, raw: &mut RawInput, barrel_down: bool) {
        let (primary_down, secondary_down, hover) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.secondary_down(), i.pointer.hover_pos()));
        self.position = hover.or(self.position);
        let mut modifiers = ctx.input(|i| i.modifiers);
        let mut lost = false;
        let mut native_secondary = false;
        // Keep the tip's release suppressed even when the barrel button is released first.
        // Otherwise that release could become a primary click and leave an unwanted dab.
        raw.events.retain(|event| {
            match event {
                Event::PointerMoved(pos) | Event::PointerButton { pos, .. } => self.position = Some(*pos),
                Event::PointerGone | Event::WindowFocused(false) => lost = true,
                _ => {}
            }
            if let Event::ModifiersChanged(current) = event {
                modifiers = *current;
            }
            if let Event::PointerButton { button, pressed, modifiers: current, .. } = event {
                modifiers = *current;
                if *button == PointerButton::Secondary {
                    native_secondary = true;
                }
                if *button == PointerButton::Primary {
                    if *pressed {
                        self.suppressed_contact = barrel_down || self.down;
                    }
                    let suppress = self.suppressed_contact;
                    if !*pressed {
                        self.suppressed_contact = false;
                    }
                    return !suppress;
                }
            }
            true
        });
        // Drivers which already deliver right-clicks need no additional events. Do not
        // interrupt a primary stroke which began before the barrel button was pressed.
        let down = barrel_down && !lost && (!primary_down || self.suppressed_contact || self.down);
        if native_secondary {
            self.down = false;
        } else if down != self.down
            && (!down || !secondary_down)
            && let Some(pos) = self.position
        {
            raw.events.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed: down, modifiers });
            self.down = down;
        }
        if lost {
            self.suppressed_contact = false;
            self.position = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PenButton;
    use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Sense, pos2, vec2};

    fn button(button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos: pos2(50.0, 50.0), button, pressed, modifiers: Modifiers::NONE }
    }

    /// Exercise egui's actual click routing, including button release before tip lift.
    #[test]
    fn barrel_click_and_tip_contact_are_secondary_and_never_primary() {
        for contact in [false, true] {
            let ctx = Context::default();
            let mut pen = PenButton::default();
            let mut secondary = 0;
            let mut primary = 0;
            let frames = [
                (false, vec![]),
                (false, vec![Event::PointerMoved(pos2(50.0, 50.0))]),
                (true, if contact { vec![button(PointerButton::Primary, true)] } else { vec![] }),
                (true, vec![]),
                (false, vec![]),
                (false, if contact { vec![button(PointerButton::Primary, false)] } else { vec![] }),
            ];
            for (down, events) in frames {
                let mut raw = RawInput { events, ..Default::default() };
                pen.normalize(&ctx, &mut raw, down);
                ctx.run_ui(raw, |ui| {
                    let r = ui.allocate_rect(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), Sense::click_and_drag());
                    secondary += usize::from(r.secondary_clicked());
                    primary += usize::from(r.clicked());
                })
                .textures_delta
                .clear();
            }
            assert_eq!(secondary, 1, "contact={contact}");
            assert_eq!(primary, 0, "contact={contact}");
            assert!(!ctx.input(|i| i.pointer.any_down()));
        }
    }

    #[test]
    fn mouse_clicks_and_native_secondary_events_are_unchanged() {
        for (down, button_kind) in [(false, PointerButton::Primary), (true, PointerButton::Secondary)] {
            let ctx = Context::default();
            let mut pen = PenButton::default();
            let expected = vec![button(button_kind, true), button(button_kind, false)];
            let mut raw = RawInput { events: expected.clone(), ..Default::default() };
            pen.normalize(&ctx, &mut raw, down);
            assert_eq!(raw.events, expected);
        }
    }

    #[test]
    fn barrel_button_does_not_interrupt_an_existing_primary_stroke() {
        let ctx = Context::default();
        let mut pen = PenButton::default();
        ctx.run_ui(RawInput { events: vec![button(PointerButton::Primary, true)], ..Default::default() }, |_| {}).textures_delta.clear();
        let mut raw = RawInput::default();
        pen.normalize(&ctx, &mut raw, true);
        assert!(raw.events.is_empty(), "no secondary press during a primary stroke");
        let release = button(PointerButton::Primary, false);
        let mut raw = RawInput { events: vec![release.clone()], ..Default::default() };
        pen.normalize(&ctx, &mut raw, true);
        assert_eq!(raw.events, vec![release], "the original primary stroke still gets its release");
    }

    #[test]
    fn barrel_resize_keeps_alt_modifier_and_the_next_tip_click_is_primary() {
        let ctx = Context::default();
        let mut pen = PenButton::default();
        let mut press = button(PointerButton::Primary, true);
        if let Event::PointerButton { modifiers, .. } = &mut press {
            *modifiers = Modifiers::ALT;
        }
        let mut raw = RawInput { events: vec![press], ..Default::default() };
        pen.normalize(&ctx, &mut raw, true);
        assert!(matches!(raw.events.as_slice(), [Event::PointerButton { button: PointerButton::Secondary, pressed: true, modifiers, .. }] if modifiers.alt));
        let mut raw = RawInput { events: vec![button(PointerButton::Primary, false)], ..Default::default() };
        pen.normalize(&ctx, &mut raw, false);
        assert!(matches!(raw.events.as_slice(), [Event::PointerButton { button: PointerButton::Secondary, pressed: false, .. }]));
        let expected = vec![button(PointerButton::Primary, true), button(PointerButton::Primary, false)];
        let mut raw = RawInput { events: expected.clone(), ..Default::default() };
        pen.normalize(&ctx, &mut raw, false);
        assert_eq!(raw.events, expected);
    }
    #[test]
    fn focus_loss_releases_secondary_even_when_the_physical_button_stays_down() {
        let ctx = Context::default();
        let mut pen = PenButton::default();
        let mut raw = RawInput { events: vec![Event::PointerMoved(pos2(50.0, 50.0))], ..Default::default() };
        pen.normalize(&ctx, &mut raw, true);
        assert!(matches!(raw.events.last(), Some(Event::PointerButton { button: PointerButton::Secondary, pressed: true, .. })));
        let mut raw = RawInput { events: vec![Event::WindowFocused(false)], ..Default::default() };
        pen.normalize(&ctx, &mut raw, true);
        assert!(matches!(raw.events.last(), Some(Event::PointerButton { button: PointerButton::Secondary, pressed: false, .. })));
    }
}
