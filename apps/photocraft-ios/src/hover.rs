//! Coalesced Pencil proximity input. Hover never generates button/paint events.
#[cfg(target_os = "ios")]
use eframe::egui;

#[derive(Default)]
pub struct HoverInput {
    pending: Option<Option<[f64; 2]>>,
    primary_down: bool,
}

impl HoverInput {
    pub fn update(&mut self, position: Option<[f64; 2]>) {
        self.pending = Some(position);
    }

    pub fn drain(&mut self, input: &mut egui::RawInput, zoom: f32) {
        let mut contact = self.primary_down;
        for event in &input.events {
            match event {
                egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed, .. } => {
                    self.primary_down = *pressed;
                    contact = true;
                }
                egui::Event::Touch { .. } => contact = true,
                _ => {}
            }
        }
        let Some(position) = self.pending.take() else { return };
        // Real touch input owns the pointer throughout a stroke, including its release frame.
        if contact {
            return;
        }
        let event = position
            .and_then(|[x, y]| {
                let zoom = if zoom.is_finite() && zoom > 0.0 { f64::from(zoom) } else { 1.0 };
                let (x, y) = ((x / zoom) as f32, (y / zoom) as f32);
                (x.is_finite() && y.is_finite()).then_some(egui::Event::PointerMoved(egui::pos2(x, y)))
            })
            .unwrap_or(egui::Event::PointerGone);
        input.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_moves_without_painting_and_leaves_without_sticking() {
        let ctx = egui::Context::default();
        let mut hover = HoverInput::default();
        hover.update(Some([20.0, 40.0]));
        hover.update(Some([60.0, 100.0]));
        let mut input = egui::RawInput::default();
        hover.drain(&mut input, 2.0);
        assert_eq!(input.events, vec![egui::Event::PointerMoved(egui::pos2(30.0, 50.0))]);
        ctx.run_ui(input, |ui| {
            ui.input(|i| {
                assert_eq!(i.pointer.hover_pos(), Some(egui::pos2(30.0, 50.0)));
                assert!(!i.pointer.any_down());
                assert!(!i.pointer.any_pressed());
            });
        })
        .drop_without_applying_deltas();
        hover.update(None);
        let mut input = egui::RawInput::default();
        hover.drain(&mut input, 1.0);
        ctx.run_ui(input, |ui| assert_eq!(ui.input(|i| i.pointer.hover_pos()), None)).drop_without_applying_deltas();
    }

    #[test]
    fn hover_does_not_override_contact_or_release() {
        let mut hover = HoverInput::default();
        for pressed in [true, false] {
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::PointerButton {
                pos: egui::pos2(1.0, 2.0),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
            hover.update(Some([99.0, 99.0]));
            hover.drain(&mut input, 1.0);
            assert_eq!(input.events.len(), 1);
            if pressed {
                let mut held = egui::RawInput::default();
                hover.update(None);
                hover.drain(&mut held, 1.0);
                assert!(held.events.is_empty());
            }
        }
    }

    #[test]
    fn invalid_coordinates_clear_hover() {
        let mut hover = HoverInput::default();
        hover.update(Some([f64::NAN, 0.0]));
        let mut input = egui::RawInput::default();
        hover.drain(&mut input, 0.0);
        assert_eq!(input.events, vec![egui::Event::PointerGone]);
    }
}
