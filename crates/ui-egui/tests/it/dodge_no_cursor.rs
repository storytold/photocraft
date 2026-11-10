//! #12817: with the Dodge tool the canvas shows the "no" cursor in the documents the tool refuses
//! (Indexed Color and Bitmap), and its usual brush cursor in the modes it tones.

use egui_kittest::Harness;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};
use photocraft_engine::Session;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::state::Tool;

/// The cursor a `mode` document shows with the Dodge tool over the canvas centre.
fn cursor(mode: ColorMode) -> egui::CursorIcon {
    let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_max_steps(8).with_accessibility_check(false).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = Session::new();
        s.add_document(Document::with_background("dodge", Size::new(400, 300), mode, SampleType::U8, Color::rgb(0.5, 0.5, 0.5)), None);
        let mut app = PhotocraftApp::new(s, Default::default());
        app.ui.tool = Tool::Dodge;
        app
    });
    h.run_steps(4);
    h.event(egui::Event::PointerMoved(egui::pos2(400.0, 300.0)));
    h.run_steps(3);
    h.output().platform_output.cursor_icon
}

#[test]
fn dodge_shows_the_no_cursor_in_indexed_and_bitmap_documents() {
    assert_eq!(cursor(ColorMode::Indexed), egui::CursorIcon::NotAllowed);
    assert_eq!(cursor(ColorMode::Bitmap), egui::CursorIcon::NotAllowed);
}

#[test]
fn dodge_keeps_its_brush_cursor_in_the_modes_it_tones() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        assert_ne!(cursor(mode), egui::CursorIcon::NotAllowed, "{mode:?} keeps the brush cursor");
    }
}
