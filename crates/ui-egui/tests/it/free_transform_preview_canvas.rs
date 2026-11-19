//! #15546: while Free Transform is open, the canvas shows the layer being transformed inside its box.
//!
//! The canvas hides that layer from the document's picture and draws it as the transform preview. The preview
//! joins the canvas's picture slot, and the document's picture is prepared after the canvas controls, so a preview
//! added with the controls lay under the document: only the layers below showed inside the box. These tests drive
//! the real app offscreen on wgpu, on the GPU canvas and on the CPU one, start Free Transform on a red layer over a
//! white background, and look at the rendered pixels inside the box, before and during a drag that moves it.
//! Skips when no GPU adapter exists (like `drag_preview_canvas.rs`).

use craft::behavior::kurbo::Point;
use egui::{Modifiers, PointerButton, Pos2};
use photocraft_geom::Rect;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The red square on the layer, in document pixels: x 100..200, y 100..200 of a 400 × 300 document.
fn square() -> Rect {
    Rect::new(100, 100, 200, 200)
}

fn harness(gpu_canvas: bool) -> Option<Harness> {
    // Only a panic before the app callback runs (no adapter, no render state) is a skip; one from the app's own setup
    // is a failure.
    let entered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = entered.clone();
    let built = std::panic::catch_unwind(move || {
        egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 640.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
            if gpu_canvas && let Some(rs) = cc.wgpu_render_state.as_ref() {
                app.set_wgpu(rs.clone());
            }
            app
        })
    });
    match built {
        Ok(h) => Some(h),
        Err(panic) if entered.load(std::sync::atomic::Ordering::SeqCst) => std::panic::resume_unwind(panic),
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

/// A white 400 × 300 document at 100 % with a layer holding the red square, active, and the Move tool.
fn setup(h: &mut Harness) {
    {
        let app = h.state_mut();
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
        app.run("layer.new.layer", json!({})).expect("new layer");
        app.session
            .edit("paint", |doc, active| {
                let id = active.expect("the new layer is active");
                let surface = doc.layer_mut(id).and_then(|l| l.surface_mut()).expect("a raster layer");
                surface.fill_rect(square(), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .expect("paint");
        app.sync_views();
        app.ui.extras.rulers = false;
    }
    control(h, "ui.set", json!({"tool": "move", "zoom": 1.0, "center": [200, 150]}));
    h.run_steps(3);
}

/// Screen point (egui points, one physical pixel each here) of document point `(x, y)`.
fn screen(h: &Harness, x: f64, y: f64) -> Pos2 {
    photocraft_ui_egui::canvas::active_view(h.state()).expect("an active document view").to_screen(Point::new(x, y))
}

fn pointer_move(h: &mut Harness, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.step();
}

fn button(h: &mut Harness, at: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

/// The rendered frame's RGB at document point `(x, y)`.
fn rgb_at(h: &mut Harness, x: f64, y: f64) -> [u8; 3] {
    let p = screen(h, x, y);
    let img = h.render().expect("render");
    let px = img.get_pixel(p.x.floor() as u32, p.y.floor() as u32);
    [px[0], px[1], px[2]]
}

fn is_red(c: [u8; 3]) -> bool {
    c[0] > 200 && c[1] < 60 && c[2] < 60
}

fn is_white(c: [u8; 3]) -> bool {
    c.iter().all(|&v| v > 230)
}

fn shows_the_layer_inside_the_box(gpu_canvas: bool) {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness(gpu_canvas) else { return };
    setup(&mut h);
    assert!(is_red(rgb_at(&mut h, 150.5, 150.5)), "the layer shows before the transform");

    control(&mut h, "ui.menu.invoke", json!({"id": "edit.freeTransform"}));
    assert!(h.state().ui.transform.is_some(), "Free Transform is open");
    let inside = rgb_at(&mut h, 150.5, 150.5);
    assert!(is_red(inside), "the layer shows inside the box once Free Transform opens, not the background: {inside:?}");

    // Drag the box 60 px right from its middle and hold: the preview follows the pointer. The samples sit off the
    // middle row, where the smart guide of the centred box is drawn.
    let (from, to) = (screen(&h, 150.0, 150.0), screen(&h, 210.0, 150.0));
    pointer_move(&mut h, from);
    button(&mut h, from, true);
    for i in 1..=6 {
        pointer_move(&mut h, from + (to - from) * (i as f32 / 6.0));
    }
    let moved = rgb_at(&mut h, 230.5, 130.5);
    assert!(is_red(moved), "the moved square shows where the drag took it: {moved:?}");
    let vacated = rgb_at(&mut h, 120.5, 130.5);
    assert!(is_white(vacated), "the place the square left shows the background: {vacated:?}");
    button(&mut h, to, false);
    let released = rgb_at(&mut h, 230.5, 130.5);
    assert!(is_red(released), "the moved square still shows after the release: {released:?}");
}

#[test]
fn free_transform_shows_the_layer_inside_its_box_on_the_gpu_canvas() {
    shows_the_layer_inside_the_box(true);
}

#[test]
fn free_transform_shows_the_layer_inside_its_box_on_the_cpu_canvas() {
    shows_the_layer_inside_the_box(false);
}

/// Escape after a move puts the square back in the frame that cancels: the cancelled preview's moving pixels are not
/// painted over the restored document where the box had taken them.
fn cancelling_shows_no_moved_pixels(gpu_canvas: bool) {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness(gpu_canvas) else { return };
    setup(&mut h);
    control(&mut h, "ui.menu.invoke", json!({"id": "edit.freeTransform"}));
    let (from, to) = (screen(&h, 150.0, 150.0), screen(&h, 210.0, 150.0));
    pointer_move(&mut h, from);
    button(&mut h, from, true);
    for i in 1..=6 {
        pointer_move(&mut h, from + (to - from) * (i as f32 / 6.0));
    }
    button(&mut h, to, false);
    assert!(is_red(rgb_at(&mut h, 230.5, 130.5)), "the box moved the square");

    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    assert!(h.state().ui.transform.is_none(), "Escape ended Free Transform");
    let moved = rgb_at(&mut h, 230.5, 130.5);
    assert!(is_white(moved), "the cancelling frame shows no moved pixels: {moved:?}");
    let back = rgb_at(&mut h, 150.5, 130.5);
    assert!(is_red(back), "the square is back in its place: {back:?}");
}

#[test]
fn cancelling_free_transform_shows_no_moved_pixels_on_the_gpu_canvas() {
    cancelling_shows_no_moved_pixels(true);
}

#[test]
fn cancelling_free_transform_shows_no_moved_pixels_on_the_cpu_canvas() {
    cancelling_shows_no_moved_pixels(false);
}
