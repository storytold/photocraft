//! UIKit points → egui logical points, without changing pointer coordinates.

#[cfg(target_os = "ios")]
use eframe::egui;

/// egui 0.36 creates its root Ui over viewport_rect, not content_rect.
/// Constrain the editor explicitly; retain the original coordinate system for pen input.
pub fn show_editor<R>(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let rect = ui.ctx().content_rect();
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(rect));
        contents(ui)
    })
    .inner
}

pub fn logical_insets(insets: [f64; 4], zoom: f32) -> [f32; 4] {
    let zoom = if zoom.is_finite() && zoom > 0.0 { f64::from(zoom) } else { 1.0 };
    insets.map(|value| {
        let value = value / zoom;
        if value.is_finite() && value > 0.0 { value.min(f64::from(f32::MAX)) as f32 } else { 0.0 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_panels_respect_safe_area_after_rotation() {
        let ctx = egui::Context::default();
        for (size, top, bottom) in [(egui::vec2(1210.0, 834.0), 24.0, 20.0), (egui::vec2(834.0, 1210.0), 24.0, 20.0)] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                safe_area_insets: Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top, bottom, left: 0.0, right: 0.0 })),
                ..Default::default()
            };
            let output = ctx.run_ui(input, |root| {
                show_editor(root, |ui| {
                    assert_eq!(ui.max_rect().top(), top);
                    assert_eq!(ui.clip_rect().bottom(), size.y - bottom);
                    let panel = egui::Panel::top("menu").show(ui, |ui| {
                        ui.label("File Edit Image");
                    });
                    assert_eq!(panel.response.rect.top(), top);
                });
            });
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn reserves_status_bar_and_home_indicator() {
        assert_eq!(logical_insets([24.0, 0.0, 0.0, 20.0], 1.0), [24.0, 0.0, 0.0, 20.0]);
    }

    #[test]
    fn respects_ui_zoom_without_retina_double_scaling() {
        assert_eq!(logical_insets([24.0, 8.0, 4.0, 20.0], 2.0), [12.0, 4.0, 2.0, 10.0]);
    }

    #[test]
    fn refreshes_for_rotation_and_zero_insets() {
        assert_eq!(logical_insets([0.0, 24.0, 24.0, 20.0], 1.0), [0.0, 24.0, 24.0, 20.0]);
        assert_eq!(logical_insets([0.0; 4], 1.0), [0.0; 4]);
    }

    #[test]
    fn invalid_values_cannot_poison_layout() {
        assert_eq!(logical_insets([f64::NAN, -1.0, f64::INFINITY, 20.0], f32::NAN), [0.0, 0.0, 0.0, 20.0]);
        assert_eq!(logical_insets([24.0; 4], 0.0), [24.0; 4]);
    }
}
