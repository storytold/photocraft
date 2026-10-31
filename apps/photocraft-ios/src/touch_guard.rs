//! iPad canvas: Pencil draws, fingers navigate. Keep raw touches for egui pinch zoom.
#[cfg(target_os = "ios")]
use eframe::egui;

/// Convert a screen-space two-finger translation to the document camera offset.
pub fn pan_offset(delta: egui::Vec2, zoom: f32, pixels_per_point: f32, rotation: f32) -> Option<egui::Vec2> {
    if !delta.is_finite() || !zoom.is_finite() || zoom <= 0.0 || !pixels_per_point.is_finite() || pixels_per_point <= 0.0 || !rotation.is_finite() {
        return None;
    }
    let (sin, cos) = rotation.to_radians().sin_cos();
    let offset = egui::vec2(cos * delta.x + sin * delta.y, -sin * delta.x + cos * delta.y) * (pixels_per_point / zoom);
    offset.is_finite().then_some(offset)
}

#[derive(Default)]
pub struct TouchGuard {
    pointer: Option<(egui::TouchDeviceId, egui::TouchId)>,
    suppress: bool,
}

impl TouchGuard {
    pub fn filter(&mut self, input: &mut egui::RawInput, canvas: egui::Rect) {
        input.events.retain(|event| {
            match event {
                egui::Event::Touch { device_id, id, phase, pos, force } => {
                    let key = (*device_id, *id);
                    match phase {
                        egui::TouchPhase::Start if self.pointer.is_none() => {
                            self.pointer = Some(key);
                            // On iPad, winit supplies calibrated force for Pencil, and None
                            // for fingers. Keep even zero-pressure Pencil contact as drawing.
                            self.suppress = force.is_none() && canvas.contains(*pos);
                        }
                        egui::TouchPhase::End | egui::TouchPhase::Cancel if self.pointer == Some(key) => {
                            self.pointer = None;
                            // Keep suppression through the following synthetic release.
                        }
                        _ => {}
                    }
                    true
                }
                egui::Event::PointerButton { button: egui::PointerButton::Primary, .. } => !self.suppress,
                egui::Event::PointerGone => {
                    self.pointer = None;
                    self.suppress = false;
                    true
                }
                _ => true,
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pan_tracks_display_scale_zoom_and_rotated_canvas() {
        assert_eq!(pan_offset(egui::vec2(20.0, 10.0), 2.0, 2.0, 0.0), Some(egui::vec2(20.0, 10.0)));
        let turned = pan_offset(egui::vec2(10.0, 0.0), 1.0, 1.0, 90.0).unwrap();
        assert!(turned.x.abs() < 0.001 && (turned.y + 10.0).abs() < 0.001);
        assert!(pan_offset(egui::Vec2::ZERO, 0.0, 2.0, 0.0).is_none());
    }
    fn frame(id: u64, phase: egui::TouchPhase, force: Option<f32>, pos: egui::Pos2, pressed: bool) -> egui::RawInput {
        egui::RawInput {
            events: vec![
                egui::Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(id), phase, pos, force },
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
            ],
            ..Default::default()
        }
    }
    #[test]
    fn home_screen_taps_survive_after_canvas_gesture() {
        let mut guard = TouchGuard::default();
        let p = egui::pos2(100.0, 100.0);
        let mut start = frame(1, egui::TouchPhase::Start, None, p, true);
        guard.filter(&mut start, egui::Rect::EVERYTHING);
        let mut cancelled = egui::RawInput { events: vec![egui::Event::PointerGone], ..Default::default() };
        guard.filter(&mut cancelled, egui::Rect::EVERYTHING);
        let mut tap = frame(2, egui::TouchPhase::Start, None, p, true);
        guard.filter(&mut tap, egui::Rect::NOTHING);
        assert!(tap.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. })));
    }
    #[test]
    fn first_finger_cannot_paint_before_second_arrives() {
        let canvas = egui::Rect::from_min_max(egui::pos2(50.0, 50.0), egui::pos2(500.0, 500.0));
        let mut guard = TouchGuard::default();
        for (id, phase, down) in
            [(1, egui::TouchPhase::Start, true), (2, egui::TouchPhase::Start, true), (1, egui::TouchPhase::Move, true), (1, egui::TouchPhase::End, false)]
        {
            let mut input = frame(id, phase, None, egui::pos2(100.0, 100.0), down);
            guard.filter(&mut input, canvas);
            assert!(!input.events.iter().any(|e| matches!(e, egui::Event::PointerButton { .. })));
            assert!(input.events.iter().any(|e| matches!(e, egui::Event::Touch { .. })));
        }
        for (force, pos) in [(Some(0.0), egui::pos2(100.0, 100.0)), (None, egui::pos2(10.0, 10.0))] {
            let mut input = frame(3, egui::TouchPhase::Start, force, pos, true);
            guard.filter(&mut input, canvas);
            assert!(input.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. })));
            let mut end = frame(3, egui::TouchPhase::End, force, pos, false);
            guard.filter(&mut end, canvas);
        }
    }
}
