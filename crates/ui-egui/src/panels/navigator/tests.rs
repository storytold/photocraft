//! The Navigator panel on the whole app: its zoom row shows the canvas's eased camera, and its buttons
//! follow General › Animated Zoom.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;

/// The app with a 200 × 100 document at 100 % and the Navigator shown, in 60 fps steps.
fn whole_app(animated_zoom: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut session = photocraft_engine::Session::new();
        session.edit_prefs(|p| p.general.auto_show_home_screen = false);
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), session, crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("prefs.set", json!({"values": {"general.animatedZoom": animated_zoom}})).unwrap();
        crate::panel_docking::show_panel(&mut app, "navigator", Some(true)).unwrap();
        app.sync_views();
        app
    });
    // The harness sets egui's animation time to zero, which turns the framework's motion off; the ease needs it on.
    h.ctx.all_styles_mut(|style| style.animation_time = egui::Style::default().animation_time);
    h.run_steps(6);
    let view = &mut h.state_mut().ui.views[0];
    view.clear_pending();
    view.zoom = 1.0;
    view.center = [100.0, 50.0];
    h.run_steps(4);
    h
}

/// The percentages the fields named Zoom show.
// PLAN: assumes the Navigator's zoom field and the status bar's carry the accessible name "Zoom" and show the
// percentage as their value (the framework's `NavigatorZoomRow` words are not in this tree).
fn percents(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<f32> {
    h.get_all_by_label("Zoom")
        .filter_map(|node| node.accesskit_node().value())
        .filter_map(|value| value.trim().trim_end_matches('%').trim().parse::<f32>().ok())
        .collect()
}

/// The Navigator's zoom row follows the view the canvas shows: during the canvas's zoom ease that is the
/// eased camera, not the target zoom.
#[test]
fn navigator_viewport_follows_the_canvas_zoom_ease() {
    let mut h = whole_app(true);
    assert!(percents(&h).iter().any(|p| (p - 100.0).abs() < 0.5), "{:?}", percents(&h));
    let view = &mut h.state_mut().ui.views[0];
    view.request_zoom_animation(true);
    view.zoom = 2.0;
    // The ease's length belongs to the framework's motion settings: look at every frame for one between the start
    // and the target.
    let mut seen = Vec::new();
    let midway = (0..30).any(|_| {
        h.run_steps(1);
        let row = percents(&h);
        let between = row.iter().any(|p| *p > 100.5 && *p < 199.5);
        seen.push(row);
        between
    });
    assert!(midway, "the row shows the eased zoom while the canvas eases: {seen:?}");
    h.run_steps(90);
    assert!(percents(&h).iter().any(|p| (p - 200.0).abs() < 0.5), "the ease ends on the target: {:?}", percents(&h));
}
