use egui::{Modifiers, Shape, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, dock::Group, state::ColorPanelMode};
use serde_json::json;

/// Whether this frame marks the picked point inside the ramp's visible area.
fn marked(h: &Harness<'_, PhotocraftApp>, point: egui::Pos2) -> bool {
    h.output()
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, Shape::Circle(c) if c.center.distance(point) < 0.5 && c.stroke.width > 0.0 && s.clip_rect.contains(point)))
}

/// A ramp pick remains marked after release in every slider-based Color panel mode.
#[test]
fn slider_ramp_keeps_the_picked_position_visible() {
    for mode in ColorPanelMode::ALL.into_iter().filter(|mode| mode.is_sliders()) {
        // Unrelated shell controls lack accessible names; this test checks the ramp marker.
        let mut h = Harness::builder().with_accessibility_check(false).with_size(vec2(1200.0, 800.0)).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
            app.ui.panels.color = true;
            app.ui.dock_tabs.color = 0;
            // Keep the whole ramp visible instead of clicking below the clipped panel body.
            app.ui.dock.heights.insert(Group::Color, 360.0);
            app.ui.color_panel.mode = mode;
            app.ui.color_panel.background = false;
            app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
            app
        });
        h.run_steps(3);
        let ramp = h.get_by_label("Color Ramp").rect();
        let point = ramp.center();
        h.get_by_label("Color Ramp").click();
        h.run_steps(2);
        h.event(egui::Event::PointerGone);
        h.run_steps(2);
        assert_ne!(h.state().session.tools.foreground, [0.0, 0.0, 0.0, 1.0], "{mode:?}: the ramp selected a color");
        assert!(marked(&h, point), "{mode:?}: the picked ramp position has a visible indicator after release");

        h.state_mut().run("tools.setColors", json!({"foreground": [0.2, 0.3, 0.4, 1.0]})).unwrap();
        h.run_steps(2);
        assert!(!marked(&h, point), "{mode:?}: another color must not retain the old marker");
        h.get_by_label("Color Ramp").click();
        h.run_steps(2);
        assert!(marked(&h, point));
        h.state_mut().ui.color_panel.web_safe_ramp = true;
        h.run_steps(2);
        assert!(!marked(&h, point), "{mode:?}: a different ramp must not retain the old marker");
        h.state_mut().ui.color_panel.web_safe_ramp = false;
        h.run_steps(2);

        let foreground = h.state().session.tools.foreground;
        let background = h.state().session.tools.background;
        h.get_by_label("Color Ramp").click_modifiers(Modifiers::ALT);
        h.run_steps(2);
        assert_eq!(h.state().session.tools.foreground, foreground, "Alt keeps the foreground");
        assert_ne!(h.state().session.tools.background, background, "Alt still picks the background");

        let colors = (h.state().session.tools.foreground, h.state().session.tools.background);
        let ramp = h.state().ui.color_panel.ramp;
        h.get_by_label("Color Ramp").click_modifiers(Modifiers::SHIFT);
        h.run_steps(2);
        assert_ne!(h.state().ui.color_panel.ramp, ramp, "Shift still cycles the ramp");
        assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background), colors, "Shift picks no color");
        assert!(!marked(&h, point), "{mode:?}: the cycled ramp has no stale marker");
    }
}
