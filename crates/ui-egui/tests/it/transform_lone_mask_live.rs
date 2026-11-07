//! #9650: while Free Transform moves a targeted unlinked layer mask, the canvas shows the layer
//! masked by the mask where the box puts it, as Photoshop does, not the mask's values in grey
//! over the unmasked layer.
//!
//! The real app runs offscreen on wgpu with real egui pointer events. Skips when no GPU adapter
//! exists (like `drag_preview_canvas.rs`). Set `PHOTOCRAFT_TRANSFORM_MASK_EVIDENCE_DIR` to save
//! the frames as PNGs.

use egui::{Modifiers, PointerButton, Pos2};
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::canvas::ViewXform;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

fn harness() -> Option<Harness> {
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 640.0)).with_max_steps(64).wgpu().build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.session.prefs.edit(|p| p.interface.language = "en".into());
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

fn press(h: &mut Harness, p: Pos2, pressed: bool) {
    h.event(egui::Event::PointerMoved(p));
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

/// The screen colours at document points `at` (y 175) in a frame of the app, saved as
/// `transform-mask-{name}.png` when evidence is asked for.
fn colors(h: &mut Harness, name: &str, at: &[f32]) -> Vec<[u8; 3]> {
    let xf = ViewXform::active(h.state()).expect("a view");
    let ppp = h.ctx.pixels_per_point();
    let img = h.render().expect("render");
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_TRANSFORM_MASK_EVIDENCE_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("evidence directory");
        img.save(dir.join(format!("transform-mask-{name}.png"))).expect("save evidence");
    }
    at.iter()
        .map(|x| {
            let p = xf.to_screen(*x, 175.0);
            let px = img.get_pixel((p.x * ppp) as u32, (p.y * ppp) as u32).0;
            [px[0], px[1], px[2]]
        })
        .collect()
}

/// Checks the frame `name`: white (hidden by the mask) or blue at each x.
fn check(h: &mut Harness, name: &str, want: &[(f32, bool)]) {
    let xs: Vec<f32> = want.iter().map(|w| w.0).collect();
    for ((x, hidden), c) in want.iter().zip(colors(h, name, &xs)) {
        assert!(if *hidden { white(c) } else { blue(c) }, "{name}: x {x} reads {c:?}");
    }
}

fn white(c: [u8; 3]) -> bool {
    c.iter().all(|v| *v > 235)
}

fn blue(c: [u8; 3]) -> bool {
    c[2] > 180 && c[0] < 80 && c[1] < 80
}

#[test]
fn a_lone_mask_moves_live_through_its_layer() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness() else { return };
    h.run_steps(4);
    {
        // A blue layer over 100..300 × 100..250 on white, its unlinked mask hiding x 100..200.
        let app = h.state_mut();
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
        app.run("layer.new.layer", json!({})).expect("layer");
        app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).expect("select");
        app.run("edit.fill", json!({"color": "#0000ff"})).expect("fill");
        app.run("select.deselect", json!({})).expect("deselect");
        app.run("layer.layerMask.revealAll", json!({})).expect("mask");
        app.session
            .edit("Fixture", |doc, a| {
                let m = doc.layer_mut(a.expect("layer")).expect("layer").mask.as_mut().expect("mask");
                m.surface.fill_rect(photocraft_geom::Rect::new(100, 0, 200, 300), &[0.0]);
                m.linked = false;
                Ok(())
            })
            .expect("fixture");
        app.sync_views();
        app.ui.mask_target = true;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
    }
    control(&mut h, "ui.set", json!({"tool": "move", "zoom": 1.0, "center": [200, 150]}));
    control(&mut h, "ui.menu.invoke", json!({"id": "edit.freeTransform"}));
    assert_eq!(h.state().ui.transform.as_ref().and_then(|t| t.target.clone()), Some(json!("mask")), "the box moves the mask");
    // The mask hides x 100..200 when the box opens.
    check(&mut h, "open", &[(150.0, true), (250.0, false)]);
    let xf = ViewXform::active(h.state()).expect("a view");
    // Drag the box 60 px right (from below the reference point, which a press would move
    // instead): the hidden band follows it to x 160..260.
    let (from, to) = (xf.to_screen(150.0, 220.0), xf.to_screen(210.0, 220.0));
    press(&mut h, from, true);
    for i in 1..=8 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.run_steps(2);
    let moved = [(130.0, false), (180.0, true), (240.0, true), (280.0, false)];
    check(&mut h, "dragged", &moved);
    press(&mut h, to, false);
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert!(h.state().ui.transform.is_none(), "applied");
    check(&mut h, "applied", &moved);
}
