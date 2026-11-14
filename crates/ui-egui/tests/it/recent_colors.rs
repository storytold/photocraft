//! #11524: what the Eyedropper samples joins the Recently Used Colors, the history the Swatches
//! panel lists under its library (and the shape Fill and Stroke popovers show). Driven the way a
//! person drives it — real clicks on the canvas — and read back from the preferences that hold the
//! list, which is what the panel draws. Like `eyedropper_ring.rs`, these skip when no GPU adapter
//! is available.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

fn harness() -> Option<Harness> {
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 800.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                app.set_wgpu(rs.clone());
            }
            app
        })
    });
    match built {
        Ok(h) => Some(h),
        Err(_) => {
            eprintln!("skipping: no GPU adapter");
            None
        }
    }
}

fn control(h: &mut Harness, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    handle(h.state_mut(), &ctx, &req);
    h.run_steps(3);
}

/// A 60 × 20 document in three colour bands with the Eyedropper in hand, its middle band centred
/// in the view at zoom 1: one screen point from the canvas centre is one document pixel.
fn setup(h: &mut Harness) -> Pos2 {
    {
        let app = h.state_mut();
        while app.session.active().is_some() {
            app.run("file.close", json!({"discard": true})).expect("close");
        }
        app.run("file.new", json!({"width": 60, "height": 20, "background": "transparent"})).expect("new");
        for (x, fill) in [(0.0, "#ff0000"), (20.0, "#00ff00"), (40.0, "#0000ff")] {
            app.run("shape.create", json!({"kind": "rect", "rect": [x, 0.0, 20.0, 20.0], "fill": fill})).expect("shape");
        }
        app.sync_views();
    }
    control(h, "ui.set", json!({"tool": "eyedropper", "zoom": 1.0, "center": [30.0, 10.0]}));
    h.run_steps(3);
    h.state().last_canvas_rect.center()
}

fn click(h: &mut Harness, p: Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers });
        h.run_steps(1);
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

#[test]
fn sampling_with_the_eyedropper_fills_the_recent_colors() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness() else { return };
    h.run_steps(4);
    let centre = setup(&mut h);
    let recent = |h: &Harness| h.state().session.prefs().recent_colors.clone();
    assert!(recent(&h).is_empty(), "nothing has been picked yet");
    // Red left of centre, green at it, blue right of it, then red again.
    for dx in [-20.0, 0.0, 20.0, -20.0] {
        click(&mut h, centre + vec2(dx, 0.0), Modifiers::NONE);
    }
    assert_eq!(recent(&h), ["#ff0000", "#0000ff", "#00ff00"], "newest first, every colour listed once");
    // ⌥ samples the background colour; Photoshop remembers that pick too.
    click(&mut h, centre, Modifiers::ALT);
    assert_eq!(h.state().session.tools.background, [0.0, 1.0, 0.0, 1.0], "the ⌥ sample took the background");
    assert_eq!(recent(&h), ["#00ff00", "#ff0000", "#0000ff"], "and joined the history");
    // The list is what the Swatches panel and the Fill and Stroke popovers both draw, so a person
    // and an agent read one history: `recentColors` in the preferences, kept for the next run.
    let saved = serde_json::to_value(h.state().session.prefs()).expect("the preferences serialize");
    assert_eq!(saved["recentColors"], serde_json::json!(recent(&h)), "the same row next launch");
}
