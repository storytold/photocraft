#[cfg(test)]
mod tests {
    use egui::{Context, Event, Modifiers, PointerButton, RawInput, Rect, TouchDeviceId, TouchId, TouchPhase, pos2, vec2};

    fn frame(ctx: &Context, time: f64, events: Vec<Event>) {
        let _ = ctx.run_ui(
            RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))), time: Some(time), events, ..Default::default() },
            |_| {},
        );
    }
    fn down() -> Event {
        Event::PointerButton { pos: pos2(100.0, 100.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }
    }
    fn cancellation() -> Vec<Event> {
        // This is the event sequence produced by the patched egui-winit on_touch branch.
        vec![
            Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Cancel, pos: pos2(100.0, 100.0), force: None },
            Event::PointerCancelled(PointerButton::Primary),
            Event::PointerGone,
        ]
    }

    #[test]
    fn rejected_touch_releases_the_button_without_clicking() {
        let ctx = Context::default();
        frame(&ctx, 0.0, vec![Event::PointerMoved(pos2(100.0, 100.0)), down()]);
        assert!(ctx.input(|i| i.pointer.primary_down()));
        frame(&ctx, 0.01, cancellation());
        ctx.input(|i| {
            assert!(!i.pointer.primary_down());
            assert!(i.pointer.primary_released());
            assert!(!i.pointer.any_click());
        });
    }

    #[test]
    fn pen_can_press_in_the_same_frame_as_palm_cancellation() {
        let ctx = Context::default();
        frame(&ctx, 0.0, vec![down()]);
        let mut events = cancellation();
        events.push(down());
        frame(&ctx, 0.01, events);
        ctx.input(|i| {
            assert!(i.pointer.primary_down());
            assert!(i.pointer.primary_pressed());
            assert!(i.pointer.primary_released());
            assert!(!i.pointer.any_click());
        });
    }

    #[test]
    fn repeated_cancel_does_not_create_a_second_release() {
        let ctx = Context::default();
        frame(&ctx, 0.0, vec![down()]);
        frame(&ctx, 0.01, cancellation());
        frame(&ctx, 0.02, cancellation());
        ctx.input(|i| {
            assert!(!i.pointer.primary_down());
            assert!(!i.pointer.any_released());
            assert!(!i.pointer.any_click());
        });
    }

    #[test]
    fn pointer_gone_keeps_its_original_mouse_drag_semantics() {
        let ctx = Context::default();
        frame(&ctx, 0.0, vec![down()]);
        frame(&ctx, 0.01, vec![Event::PointerGone]);
        assert!(ctx.input(|i| i.pointer.primary_down()));
        frame(&ctx, 0.02, cancellation());
        assert!(!ctx.input(|i| i.pointer.primary_down()));
    }

    #[test]
    fn ordinary_release_still_generates_a_click() {
        let ctx = Context::default();
        frame(&ctx, 0.0, vec![down()]);
        frame(&ctx, 0.01, vec![Event::PointerButton { pos: pos2(100.0, 100.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        assert!(ctx.input(|i| i.pointer.any_click()));
    }
}
