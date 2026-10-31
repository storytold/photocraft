//! Exercise the real logic/UI callbacks to catch an accidentally blank first editor frame.

use eframe::{App as _, egui};
use photocraft_ui_egui::{PhotocraftApp, Services, i18n};

/// Run one complete UI pass without a native window or graphics device.
fn draw(app: &mut PhotocraftApp, ctx: &egui::Context) -> egui::FullOutput {
    let mut frame = eframe::Frame::_new_kittest();
    let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))), ..Default::default() };
    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = ctx.native_pixels_per_point();
    app.raw_input_hook(ctx, &mut input);
    ctx.run_ui(input, |ui| {
        app.logic(ui.ctx(), &mut frame);
        app.ui(ui, &mut frame);
    })
}

/// App-creator preparation must make the first presentation contain editor text, with its
/// named weights already active, rather than spend that presentation installing fonts.
#[test]
fn prepared_context_draws_editor_text_on_the_first_pass() {
    let _language = i18n::language_scope(i18n::Lang::EN);
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.prepare_first_frame(&ctx);
    let output = draw(&mut app, &ctx);
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(_))), "first presentation must draw editor text");
    ctx.fonts(|fonts| {
        for name in ["medium", "semibold"] {
            assert!(fonts.families().contains(&egui::FontFamily::Name(name.into())), "missing first-pass family: {name}");
        }
    });
}

/// Hosts that cannot prepare before begin-pass retain the safe blank-then-editor fallback;
/// drawing named font families in that first unprepared pass would panic inside egui.
#[test]
fn unprepared_context_keeps_the_deferred_font_setup() {
    let _language = i18n::language_scope(i18n::Lang::EN);
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    let first = draw(&mut app, &ctx);
    assert!(!first.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(_))));
    let second = draw(&mut app, &ctx);
    assert!(second.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(_))));
}

/// Saved font size must apply to the first visible editor frame and stay stable afterward,
/// rather than briefly drawing Small or accumulating another scaling factor every pass.
#[test]
fn saved_font_size_is_active_on_the_first_pass() {
    let _language = i18n::language_scope(i18n::Lang::EN);
    let ctx = egui::Context::default();
    let services = Services { load_prefs: Some(Box::new(|| Some(r#"{"interface":{"uiFontSize":"large","language":"en"}}"#.into()))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.prepare_first_frame(&ctx);
    for pass in 1..=3 {
        let output = draw(&mut app, &ctx);
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(_))), "pass {pass}");
        ctx.fonts(|fonts| {
            let scale = fonts.definitions().font_data.get("Inter").expect("Inter installed").tweak.scale;
            assert!((scale - 16.0 / 12.0).abs() < 0.0001, "pass {pass}: scale {scale}");
        });
    }
}

/// A saved absolute UI scale must override native DPI and restored zoom before the first
/// editor draw. Its initial content rect must come from the new window, not a queued rescale
/// of the context's placeholder rect. Later passes must retain the same scale and geometry.
#[test]
fn saved_ui_scale_is_active_on_the_first_pass() {
    let _language = i18n::language_scope(i18n::Lang::EN);
    for (saved, native, restored_zoom, expected) in [("100", 2.0, 1.0, 1.0), ("150", 1.25, 1.0, 1.5), ("200", 1.0, 1.3, 2.0), ("auto", 1.5, 1.4, 1.5)] {
        let ctx = egui::Context::default();
        // eframe seeds these values before invoking its app creator.
        ctx.input_mut(|input| {
            input.raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
            input.pixels_per_point = native * restored_zoom;
        });
        ctx.options_mut(|options| options.zoom_factor = restored_zoom);
        let services = Services {
            load_prefs: Some(Box::new(move || Some(serde_json::json!({"interface": {"uiScale": saved, "language": "en"}}).to_string()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.prepare_first_frame(&ctx);
        for pass in 1..=2 {
            let output = draw(&mut app, &ctx);
            assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(_))), "{saved}, native {native}, pass {pass}: missing editor");
            assert!((output.pixels_per_point - expected).abs() < 0.0001, "{saved}, native {native}, pass {pass}: {}", output.pixels_per_point);
            assert_eq!(ctx.content_rect().size(), egui::vec2(1440.0, 900.0), "{saved}, native {native}, pass {pass}: initial viewport geometry");
        }
    }
}

/// A saved non-English language must be selected while the first editor pass is drawn,
/// rather than replacing an initial English menu on the following pass.
#[test]
fn saved_japanese_language_is_active_on_the_first_pass() {
    let _language = i18n::language_scope(i18n::Lang::EN);
    let ctx = egui::Context::default();
    let services = Services { load_prefs: Some(Box::new(|| Some(r#"{"interface":{"language":"ja"}}"#.into()))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.prepare_first_frame(&ctx);
    let output = draw(&mut app, &ctx);
    assert_eq!(app.session.prefs().interface.language, "ja");
    assert_eq!(i18n::current(), i18n::Lang::from_pref("ja"));
    assert!(
        output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains("ファイル"))),
        "the first editor pass must contain the translated File menu"
    );
}

/// Replacing a host context must register resources for that new context before its first
/// pass, even when the app has already rendered on another context.
#[test]
fn replacement_context_also_draws_its_first_pass() {
    let _language = i18n::language_scope(i18n::Lang::EN);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    for _ in 0..2 {
        let ctx = egui::Context::default();
        app.prepare_first_frame(&ctx);
        let output = draw(&mut app, &ctx);
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(_))));
    }
}
