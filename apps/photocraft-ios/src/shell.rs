//! iPad-only integration; the shared editor and its input coordinates stay unchanged.
use eframe::{App, Frame, egui};
use photocraft_ui_egui::PhotocraftApp;

pub struct IosShell(pub PhotocraftApp, pub crate::touch_guard::TouchGuard);

impl App for IosShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut Frame) {
        crate::scene::poll_file_picker(ctx);
        self.0.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        let ctx = ui.ctx().clone();
        // egui's zoom_delta is consumed by the editor, but its translation_delta wasn't.
        // Apply translation independently so dragging and pinching can happen together.
        if let Some(gesture) = ctx.input(|i| i.multi_touch())
            && gesture.num_touches == 2
            && self.0.last_canvas_rect.contains(gesture.start_pos)
            && !self.0.ui.chrome.shows_home(self.0.session.documents().len(), self.0.session.prefs().general.auto_show_home_screen)
            && let Some(index) = self.0.session.active_index()
        {
            let ppp = self.0.canvas_ppp();
            if let Some(view) = self.0.ui.views.get_mut(index)
                && let Some(delta) = crate::touch_guard::pan_offset(gesture.translation_delta, view.zoom, ppp, view.rotation)
            {
                let next = [view.center[0] - delta.x, view.center[1] - delta.y];
                if next.iter().all(|v| v.is_finite()) {
                    view.center = next;
                }
            }
        }
        crate::safe_area::show_editor(ui, |ui| self.0.ui(ui, frame));
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.0.raw_input_hook(ctx, input);
        let over_popup = input.events.iter().any(|event| {
            matches!(event, egui::Event::Touch { phase: egui::TouchPhase::Start, pos, .. }
                if ctx.layer_id_at(*pos).is_some_and(|layer| layer.order != egui::Order::Background))
        });
        // last_canvas_rect is only valid after a document canvas has actually been drawn.
        // On the Home screen its initial placeholder overlaps menus and document buttons.
        let has_canvas = !self.0.ui.chrome.shows_home(self.0.session.documents().len(), self.0.session.prefs().general.auto_show_home_screen)
            && self.0.session.active().is_some();
        self.1.filter(input, if over_popup || !has_canvas { egui::Rect::NOTHING } else { self.0.last_canvas_rect });
        crate::scene::drain_pencil_hover(input, ctx.zoom_factor());
        // winit can sample zero insets before our scene attachment, with no later resize.
        // Query the actual retained window each frame, including after rotation or resume.
        if let Some(insets) = crate::scene::safe_area_insets() {
            let [top, left, right, bottom] = crate::safe_area::logical_insets(insets, ctx.zoom_factor());
            input.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 { top, left, right, bottom }));
        }
    }
}
