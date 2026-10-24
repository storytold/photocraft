//! A keyboard caret step past the canvas edge scrolls the view toward the caret, as in
//! Photoshop: arrowing through a long block of type brings the caret back in sight. Only keys
//! do this — mouse clicks and drags never pull the camera back — and a view of an image that
//! already fits never auto-scrolls (the limits live in `canvas_autoscroll`).
//!
//! Before the change the camera kept its place and the caret ran off screen; the real app runs
//! offscreen on wgpu with real egui key events. Skips when no GPU adapter exists (like
//! `marquee_autoscroll.rs`).

use egui::{Modifiers, PointerButton, Pos2};
use photocraft_ui_egui::PhotocraftApp;
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

fn harness() -> Option<Harness> {
    use eframe::egui_wgpu::WgpuSetup;
    use eframe::wgpu;
    // Skip only when the machine has no adapter at all: enumerate the adapters the configured
    // setup would consider, and only then let kittest's render-state creation run uncaught — a
    // real device failure fails the test instead of posing as a headless machine (kittest turns
    // it into a panic).
    let setup = photocraft_ui_egui::gpu_canvas::wgpu_setup();
    let backends = match &setup {
        WgpuSetup::CreateNew(create) => create.instance_descriptor.backends,
        WgpuSetup::Existing(_) => wgpu::Backends::all(),
    };
    let instance = pollster::block_on(setup.new_instance());
    if pollster::block_on(instance.enumerate_adapters(backends)).is_empty() {
        eprintln!("skipping: no GPU adapter");
        return None;
    }
    let rs = egui_kittest::wgpu::create_render_state(setup, Default::default());
    Some(
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 640.0))
            .with_max_steps(64)
            .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs))
            .build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                if let Some(rs) = cc.wgpu_render_state.as_ref() {
                    app.set_wgpu(rs.clone());
                }
                app
            }),
    )
}

fn control(h: &mut Harness, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = photocraft_ui_egui::control::ControlRequest::new(method, params);
    photocraft_ui_egui::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(3);
}

/// 30 lines of type far past the bottom edge of the 900 × 640 canvas at 100 % with the caret on
/// its head and an inline edit session open; the camera at 100 % over the type's head.
fn setup(h: &mut Harness) -> u64 {
    let id = {
        let app = h.state_mut();
        while app.session.active().is_some() {
            app.run("file.close", json!({"discard": true})).expect("close");
        }
        app.run("file.new", json!({"width": 3000, "height": 2000, "background": "white"})).expect("new");
        app.sync_views();
        app.ui.extras.rulers = false;
        let text = "Lorem ipsum dolor sit amet.\n".repeat(120);
        let v = app.run("type.create", json!({"text": text, "size": 24, "x": 200, "y": 200})).expect("create");
        let id = v["layer"].as_u64().unwrap();
        app.session.select_layer(photocraft_doc::LayerId(id)).expect("select");
        id
    };
    control(h, "ui.set", json!({"tool": "type", "zoom": 1.0, "center": [200, 200]}));
    {
        let app = h.state_mut();
        // The inline edit session: caret at the text's head, like a keyboard session.
        app.ui.text_edit = Some(photocraft_ui_egui::state::TextEdit {
            layer: id,
            document: None,
            caret: 0,
            anchor: 0,
            session: "s".into(),
            created: false,
            mask: false,
            dragging: false,
            resize: None,
            preedit: None,
        });
    }
    h.run_steps(3);
    id
}

fn center(h: &Harness) -> [f32; 2] {
    h.state().ui.views[0].center
}

fn key(h: &mut Harness, modifiers: Modifiers, k: egui::Key) {
    h.key_press_modifiers(modifiers, k);
    h.run_steps(3);
}

fn click(h: &mut Harness, p: Pos2) {
    h.hover_at(p);
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn a_keyboard_caret_step_offscreen_scrolls_the_view_to_follow() {
    let Some(mut h) = harness() else { return };
    let _id = setup(&mut h);
    let before = center(&h);
    key(&mut h, Modifiers::COMMAND, egui::Key::End);
    let after = center(&h);
    assert!(after[1] > before[1] + 50.0, "the camera followed the caret past the bottom edge: {before:?} → {after:?}");

    // Once the caret is back in view, keys move it without moving the camera (the text kept the
    // caret near the canvas edge, so the next few steps can't shift it again).
    let settled = center(&h);
    key(&mut h, Modifiers::NONE, egui::Key::ArrowDown);
    key(&mut h, Modifiers::NONE, egui::Key::ArrowDown);
    assert_eq!(center(&h), settled, "an in-view caret move doesn't shake the camera");
}

/// Mouse clicks never pull the camera back: only keys set the follow flag.
#[test]
fn a_click_placing_the_caret_does_not_scroll() {
    let Some(mut h) = harness() else { return };
    let _id = setup(&mut h);
    let before = center(&h);
    // Start from no session: from the click itself.
    h.state_mut().ui.text_edit = None;
    h.run_steps(2);
    // The canvas centre at zoom 100 %, camera (200, 200) is the type's origin point: the click
    // starts an inline session there and never moves the camera.
    let c = h.state().last_canvas_rect.center();
    click(&mut h, c);
    assert!(h.state().ui.text_edit.is_some(), "the click has an inline edit session");
    assert!(!h.state().ui.type_caret_moved, "a click never sets the follow flag");
    assert_eq!(center(&h), before, "the camera holds its place for clicks");
}
