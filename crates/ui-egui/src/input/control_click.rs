//! Normalize macOS Control-click, and a pen press with the barrel button held (Windows, #5364),
//! into right-clicks before egui routes clicks to panels and canvas tools.
#[derive(Default)]
pub(crate) struct ControlClick {
    secondary_down: bool,
    /// The last press was a pen barrel press, until the next press (read on its release frame).
    pen_secondary: bool,
}

impl ControlClick {
    /// Was the last press a pen press with the barrel button held? A hovering hand can't hold a
    /// press as still as a mouse, so the canvas takes its release as a right-click (#5364).
    pub fn pen_secondary(&self) -> bool {
        self.pen_secondary
    }

    pub fn normalize(&mut self, events: &mut [egui::Event], macos: bool) {
        self.normalize_with_pen(events, macos, false);
    }

    /// [`Self::normalize`], where `pen_barrel` says this frame's pen press had the barrel button
    /// held. Only the press egui-winit emulates for that contact is remapped: the one following a
    /// touch start, at the touch's own position, so a mouse press in the same frame stays primary
    /// (a finger that sends no emulated press can't lend its touch start to it). A barrel press's
    /// release still maps after the barrel lets go.
    pub fn normalize_with_pen(&mut self, events: &mut [egui::Event], macos: bool, pen_barrel: bool) {
        let mut touch_start: Option<egui::Pos2> = None;
        for event in events {
            match event {
                egui::Event::Touch { phase: egui::TouchPhase::Start, pos, .. } => touch_start = Some(*pos),
                egui::Event::PointerButton { button, pressed, modifiers, pos } if *button == egui::PointerButton::Primary => {
                    if *pressed {
                        let pen_press = pen_barrel && touch_start.take().is_some_and(|t| t.distance(*pos) < 0.5);
                        // Control-Option is the existing brush resize gesture.
                        self.secondary_down = pen_press || (macos && modifiers.ctrl && !modifiers.alt && !modifiers.command);
                        self.pen_secondary = pen_press;
                    }
                    if self.secondary_down {
                        *button = egui::PointerButton::Secondary;
                    }
                    if !*pressed {
                        self.secondary_down = false;
                    }
                }
                // A mouse's own right or middle press is no pen press.
                egui::Event::PointerButton { pressed: true, .. } => self.pen_secondary = false,
                // A drag keeps the mapping when the pointer leaves the window or focus is lost: the
                // release still arrives and must match the secondary press egui already saw. The
                // next press recomputes it, so a missed release can't remap a plain click.
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton};

    fn button(pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos: egui::pos2(50.0, 50.0), button: PointerButton::Primary, pressed, modifiers }
    }

    /// The touch start egui-winit sends ahead of a pen contact's emulated press.
    fn touch_start() -> Event {
        Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase: egui::TouchPhase::Start, pos: egui::pos2(50.0, 50.0), force: None }
    }

    #[test]
    fn control_click_opens_context_menu_even_when_control_is_released_first() {
        let ctx = egui::Context::default();
        let mut state = ControlClick::default();
        let mut secondary = false;
        let mut primary = false;
        for mut events in [vec![], vec![Event::PointerMoved(egui::pos2(50.0, 50.0)), button(true, Modifiers::CTRL)], vec![button(false, Modifiers::NONE)]] {
            state.normalize(&mut events, true);
            let raw = egui::RawInput { events, ..Default::default() };
            ctx.run_ui(raw, |ui| {
                let r = ui.allocate_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0)), egui::Sense::click());
                secondary |= r.secondary_clicked();
                primary |= r.clicked();
                r.context_menu(|ui| {
                    ui.label("Layer context menu");
                });
            })
            .textures_delta
            .clear();
        }
        assert!(secondary);
        assert!(!primary);
    }

    #[test]
    fn other_platforms_and_existing_modifier_gestures_stay_primary() {
        for (macos, mods) in
            [(false, Modifiers::CTRL), (true, Modifiers::NONE), (true, Modifiers::CTRL | Modifiers::ALT), (true, Modifiers::CTRL | Modifiers::COMMAND)]
        {
            let mut events = vec![button(true, mods), button(false, mods)];
            ControlClick::default().normalize(&mut events, macos);
            assert!(events.iter().all(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, .. })));
        }
    }

    #[test]
    fn a_control_drag_out_of_the_window_and_back_releases_the_secondary_button() {
        let ctx = egui::Context::default();
        let mut state = ControlClick::default();
        let frames = [
            vec![Event::PointerMoved(egui::pos2(50.0, 50.0)), button(true, Modifiers::CTRL)],
            vec![Event::PointerGone],
            vec![Event::WindowFocused(false)],
            vec![Event::PointerMoved(egui::pos2(60.0, 60.0))],
            vec![button(false, Modifiers::NONE)],
        ];
        let mut down = Vec::new();
        for mut events in frames {
            state.normalize(&mut events, true);
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                down.push(ui.input(|i| (i.pointer.secondary_down(), i.pointer.primary_down())));
            })
            .textures_delta
            .clear();
        }
        assert_eq!(down.first(), Some(&(true, false)), "the Control press is a secondary press");
        assert_eq!(down.last(), Some(&(false, false)), "the release after re-entry ends it: no stuck button");
    }

    #[test]
    fn a_pen_barrel_tap_opens_the_context_menu_and_the_next_click_stays_primary() {
        let ctx = egui::Context::default();
        let mut state = ControlClick::default();
        let mut secondary = false;
        let mut primary = false;
        // A quick tap: press and release in one frame, the barrel known only at the press.
        for (mut events, barrel) in [
            (vec![Event::PointerMoved(egui::pos2(50.0, 50.0))], false),
            (vec![touch_start(), button(true, Modifiers::NONE), button(false, Modifiers::NONE)], true),
        ] {
            state.normalize_with_pen(&mut events, false, barrel);
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                let r = ui.allocate_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0)), egui::Sense::click());
                secondary |= r.secondary_clicked();
                primary |= r.clicked();
            })
            .textures_delta
            .clear();
        }
        assert!(secondary);
        assert!(!primary);
        let mut events = [button(true, Modifiers::NONE), button(false, Modifiers::NONE)];
        state.normalize_with_pen(&mut events, false, false);
        assert!(events.iter().all(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, .. })));
    }

    #[test]
    fn a_barrel_press_released_after_the_barrel_lets_go_still_releases_secondary() {
        let mut state = ControlClick::default();
        let mut press = [touch_start(), button(true, Modifiers::NONE)];
        state.normalize_with_pen(&mut press, false, true);
        let mut release = [button(false, Modifiers::NONE)];
        state.normalize_with_pen(&mut release, false, false);
        assert!(matches!(press[1], Event::PointerButton { button: PointerButton::Secondary, .. }));
        assert!(matches!(release[0], Event::PointerButton { button: PointerButton::Secondary, .. }));
    }

    #[test]
    fn a_finger_touch_start_does_not_remap_a_mouse_press_elsewhere() {
        let mut state = ControlClick::default();
        let finger =
            Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase: egui::TouchPhase::Start, pos: egui::pos2(300.0, 20.0), force: None };
        let mut events = [finger, button(true, Modifiers::NONE), button(false, Modifiers::NONE)];
        state.normalize_with_pen(&mut events, false, true);
        assert!(events[1..].iter().all(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, .. })));
        assert!(!state.pen_secondary());
    }

    #[test]
    fn a_mouse_press_in_the_pen_barrel_frame_stays_primary() {
        // The mouse press comes before the pen's touch start, or after the pen's own press.
        for mouse_first in [true, false] {
            let mut state = ControlClick::default();
            let mut events = if mouse_first {
                vec![button(true, Modifiers::NONE), button(false, Modifiers::NONE), touch_start(), button(true, Modifiers::NONE)]
            } else {
                vec![touch_start(), button(true, Modifiers::NONE), button(false, Modifiers::NONE), button(true, Modifiers::NONE)]
            };
            state.normalize_with_pen(&mut events, false, true);
            let pen = if mouse_first { 3 } else { 1 };
            let mouse = if mouse_first { 0 } else { 3 };
            assert!(matches!(events[pen], Event::PointerButton { button: PointerButton::Secondary, .. }), "mouse_first={mouse_first}");
            assert!(matches!(events[mouse], Event::PointerButton { button: PointerButton::Primary, .. }), "mouse_first={mouse_first}");
        }
    }

    #[test]
    fn focus_loss_does_not_remap_the_next_plain_click() {
        let mut state = ControlClick::default();
        state.normalize(&mut [button(true, Modifiers::CTRL)], true);
        state.normalize(&mut [Event::WindowFocused(false)], true);
        let mut events = [button(true, Modifiers::NONE), button(false, Modifiers::NONE)];
        state.normalize(&mut events, true);
        assert!(events.iter().all(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, .. })));
    }
}
