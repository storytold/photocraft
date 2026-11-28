//! The Eyedropper tool's options bar (#1649): Sample Size, Sample (which layers) and Show
//! Sampling Ring, as in Photoshop. The values live in `state::ToolOptions::eyedropper`, so the
//! control channel can read and set them; sampling itself is `document.sampleColor`
//! (`canvas::sampling::eyedropper_color`). The Sample Size is shared with the painting tools'
//! ⌥-click eyedropper, the dialog eyedroppers and the Info panel, as in Photoshop.

use craft::color_ui::ui::EyedropperBar;

use crate::PhotocraftApp;

/// The options bar for the Eyedropper tool.
pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let mut options = app.ui.tool_options.eyedropper;
    EyedropperBar::new(&mut options).show(ui);
    if options != app.ui.tool_options.eyedropper {
        app.ui.tool_options.eyedropper = options;
    }
}

#[cfg(test)]
mod tests {
    use craft::color_ui::ui::SampleSize;
    use serde_json::json;

    use crate::PhotocraftApp;
    use crate::canvas::{ToolEvent, tool_event};
    use crate::state::Tool;

    /// The Sample Size whose square has side `n`.
    fn size(n: u32) -> SampleSize {
        SampleSize::from_pixels(n).unwrap()
    }

    /// An app with a 40×40 white document whose top-left 3×3 pixels are black.
    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 40, "background": "white"})).unwrap();
        app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
        app.session
            .edit("paint", |doc, _| {
                let bg = doc.layers[0].surface_mut().unwrap();
                let black = photocraft_raster::from_rgba(&bg.format(), [0.0, 0.0, 0.0, 1.0]);
                bg.fill_rect(photocraft_geom::Rect::new(0, 0, 3, 3), &black);
                Ok(())
            })
            .unwrap();
        app.ui.tool = Tool::Eyedropper;
        app
    }

    fn fg(app: &PhotocraftApp) -> [f32; 4] {
        app.session.tools.foreground
    }

    fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
        tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
        tool_event(app, ToolEvent::Up { x, y }, mods);
    }

    #[test]
    fn mask_sampling_sets_gray_swatches_without_changing_pixel_sampling() {
        for depth in [8, 16, 32] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 20, "height": 20, "depth": depth, "background": "transparent"})).unwrap();
            app.session
                .edit("red", |doc, _| {
                    let surface = doc.layers[0].surface_mut().unwrap();
                    surface.fill_rect(photocraft_geom::Rect::new(0, 0, 20, 20), &photocraft_raster::from_rgba(&surface.format(), [1.0, 0.0, 0.0, 1.0]));
                    doc.layers[0].mask = Some(photocraft_doc::LayerMask::reveal_all());
                    Ok(())
                })
                .unwrap();
            app.ui.tool = Tool::Eyedropper;
            app.ui.mask_target = true;
            click(&mut app, 5.0, 5.0, egui::Modifiers::NONE);
            let gray = photocraft_color::convert::rgb_to_gray([1.0, 0.0, 0.0]);
            assert_eq!(fg(&app), [gray, gray, gray, 1.0], "depth {depth}");
            click(&mut app, 5.0, 5.0, egui::Modifiers::ALT);
            assert_eq!(app.session.tools.background, [gray, gray, gray, 1.0]);
            app.ui.mask_target = false;
            click(&mut app, 5.0, 5.0, egui::Modifiers::NONE);
            assert_eq!(fg(&app), [1.0, 0.0, 0.0, 1.0]);
        }
    }

    /// View the document at 200 %, where an N by N Average is the centred N × N square (#11751:
    /// at 100 % and below it covers (N+1) × (N+1) screen pixels).
    fn zoomed_in(app: &mut PhotocraftApp) {
        app.sync_views();
        for v in &mut app.ui.views {
            v.zoom = 2.0;
            // The first pass would otherwise fit the document to the window.
            v.clear_pending();
        }
    }

    #[test]
    fn a_click_samples_the_average_of_the_sample_size() {
        let mut app = app();
        // At 100 % a 3 by 3 Average is the 4 × 4 box from (2, 2): one black pixel of sixteen (#11751).
        app.sync_views();
        for v in &mut app.ui.views {
            v.zoom = 1.0;
            v.clear_pending();
        }
        app.ui.tool_options.eyedropper.size = size(3);
        click(&mut app, 3.5, 3.5, egui::Modifiers::NONE);
        let want = (255.0f32 * 15.0 / 16.0).round() / 255.0;
        assert_eq!(fg(&app), [want, want, want, 1.0], "3 by 3 at 100 %");
        app.ui.tool_options.eyedropper.size = size(1);
        zoomed_in(&mut app);
        // Point Sample at (3, 3): white.
        click(&mut app, 3.5, 3.5, egui::Modifiers::NONE);
        assert_eq!(fg(&app), [1.0; 4]);
        // 3 by 3 Average: one black pixel of nine, rounded to 8 bit.
        app.ui.tool_options.eyedropper.size = size(3);
        click(&mut app, 3.5, 3.5, egui::Modifiers::NONE);
        let want = (255.0f32 * 8.0 / 9.0).round() / 255.0;
        assert_eq!(fg(&app), [want, want, want, 1.0]);
        // 5 by 5 at the corner: clipped to the black 3×3.
        app.ui.tool_options.eyedropper.size = size(5);
        click(&mut app, 0.5, 0.5, egui::Modifiers::NONE);
        assert_eq!(fg(&app), [0.0, 0.0, 0.0, 1.0]);
        // ⌥-click sets the background colour, with the same size.
        app.ui.tool_options.eyedropper.size = size(3);
        click(&mut app, 3.5, 3.5, egui::Modifiers::ALT);
        assert_eq!(app.session.tools.background, [want, want, want, 1.0]);
        // The painting tools' ⌥-click eyedropper uses it too.
        app.ui.tool = Tool::Brush;
        app.ui.tool_options.eyedropper.size = size(101);
        click(&mut app, 20.0, 20.0, egui::Modifiers::ALT);
        let want = (255.0f32 * (1600.0 - 9.0) / 1600.0).round() / 255.0;
        assert_eq!(fg(&app), [want, want, want, 1.0]);
        // The dialog eyedroppers (and the Color Picker's) average the same square.
        app.ui.tool_options.eyedropper.size = size(5);
        let c = crate::canvas::composite_color(&mut app, 1.0, 1.0).unwrap();
        let want = (255.0f32 * 7.0 / 16.0).round() / 255.0;
        assert_eq!(c, [want; 3]);
    }
}
