//! Camera Raw through the real app shell and its existing agent entry point.
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::AppTheme};
use serde_json::{Value, json};

/// The open dialog's description as the agent entry point reports it.
fn described(h: &Harness<'_, Box<PhotocraftApp>>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

/// Runs the app's frames until the develop of the settings in force has landed, and returns the dialog's
/// description. The develop waits on the framework's queue until a request finishes it, as the agent entry
/// point's `waitPreview` does.
fn settled(h: &mut Harness<'_, Box<PhotocraftApp>>) -> Value {
    h.run_steps(2);
    for _ in 0..2000 {
        craft::ui::runtime::pending::finish_jobs(&h.ctx, std::time::Duration::from_secs(30));
        h.run_steps(1);
        let now = described(h);
        if now["previewPending"] == false && !now["histogram"].is_null() {
            return now;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the Camera Raw preview never finished");
}

/// A control request, then the develop it asks for.
fn request(h: &mut Harness<'_, Box<PhotocraftApp>>, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let reply = camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &params).unwrap().unwrap();
    if described(h).is_null() {
        return reply;
    }
    settled(h)
}

#[test]
fn histogram_matches_visible_preview_at_all_supported_depths_and_cancel_commit_are_reversible() {
    for mode in ["rgb", "grayscale"] {
        for depth in [8, 16, 32] {
            let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, AppTheme::pro());
                let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
                app.run("file.new", json!({"width": 16, "height": 8, "mode": mode, "depth": depth})).unwrap();
                app.run("layer.new.layer", json!({})).unwrap();
                app.run("edit.fill", json!({"color": "#808080"})).unwrap();
                app
            });
            h.run_steps(3);
            let original = h.state().session.active().unwrap().doc.clone();
            let steps = h.state().session.active().unwrap().history.past_len();
            let initial = request(&mut h, json!({}));
            assert_eq!(initial["histogram"]["samples"], 128);
            let changed = request(&mut h, json!({"ui": {"set": {"exposure": 1.0}}}));
            assert_ne!(changed["histogram"]["red"], initial["histogram"]["red"]);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), steps);
            request(&mut h, json!({"ui": {"cancel": true}}));
            assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &original), "Cancel leaves the document alone");
            request(&mut h, json!({"ui": {"set": {"exposure": 1.0}, "commit": true}}));
            assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "OK adds one step");
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &original));
            h.state_mut().run("edit.redo", json!({})).unwrap();
            let committed = request(&mut h, json!({}));
            assert_eq!(committed["histogram"]["red"], changed["histogram"]["red"], "{mode}/{depth}: graph and committed pixels disagree");
        }
    }
}
