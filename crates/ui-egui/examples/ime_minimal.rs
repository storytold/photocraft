// Native IME probe: fresh app launch, Apple Korean 2-set, type g k s -> 한.
// Synthetic egui events cannot reproduce the AppKit first-key fallback.
//! Local diagnostic only: native Korean IME with no PhotoCraft canvas/engine.
#[derive(Default)]
struct ImeProbe {
    text: String,
    focus: bool,
}
impl eframe::App for ImeProbe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.label("Korean IME minimal reproduction: type g k s using Korean input.");
        let response = ui.add(egui::TextEdit::singleline(&mut self.text).desired_width(600.0));
        if !self.focus {
            response.request_focus();
            self.focus = true;
        }
    }
}
fn main() -> eframe::Result {
    eframe::run_native("IME Minimal", eframe::NativeOptions::default(), Box::new(|_| Ok(Box::new(ImeProbe::default()))))
}
