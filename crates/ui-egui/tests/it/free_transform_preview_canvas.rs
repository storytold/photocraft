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
use egui_kittest::kittest::Queryable;
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

fn is_blue(c: [u8; 3]) -> bool {
    c[2] > 200 && c[0] < 60 && c[1] < 60
}

/// Perspective on a smart object, then Warp without a pull: the picture stays as the Perspective drew it, as the
/// reference shows it. A smart object red over blue (x 60..240, red y 90..150, blue y 150..210), its top narrowed to
/// x 126..174: the exact map puts the content's middle row at y ≈ 115, a flat grid's control points carried through
/// the move at y ≈ 126. Document point (135, 120.5) is blue in the first and red in the second.
fn warp_after_perspective_keeps_the_picture(gpu_canvas: bool) {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness(gpu_canvas) else { return };
    {
        let app = h.state_mut();
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
        app.run("layer.new.layer", json!({})).expect("new layer");
        app.session
            .edit("paint", |doc, active| {
                let id = active.expect("the new layer is active");
                let surface = doc.layer_mut(id).and_then(|l| l.surface_mut()).expect("a raster layer");
                surface.fill_rect(Rect::new(60, 90, 240, 150), &[1.0, 0.0, 0.0, 1.0]);
                surface.fill_rect(Rect::new(60, 150, 240, 210), &[0.0, 0.0, 1.0, 1.0]);
                Ok(())
            })
            .expect("paint");
        app.run("layer.smartObjects.convertToSmartObject", json!({})).expect("smart object");
        app.sync_views();
        app.ui.extras.rulers = false;
    }
    control(&mut h, "ui.set", json!({"tool": "move", "zoom": 1.0, "center": [200, 150]}));
    h.run_steps(3);
    control(&mut h, "ui.menu.invoke", json!({"id": "edit.transform.perspective"}));
    assert!(h.state().ui.transform.is_some(), "Perspective is open");
    let (from, to) = (screen(&h, 60.0, 90.0), screen(&h, 126.0, 90.0));
    pointer_move(&mut h, from);
    button(&mut h, from, true);
    for i in 1..=8 {
        pointer_move(&mut h, from + (to - from) * (i as f32 / 8.0));
    }
    button(&mut h, to, false);
    h.run_steps(4);
    let persp = rgb_at(&mut h, 135.5, 120.5);
    assert!(is_blue(persp), "the Perspective foreshortens the red top: {persp:?}");

    control(&mut h, "ui.menu.invoke", json!({"id": "edit.transform.warp"}));
    assert!(h.state().ui.transform.as_ref().is_some_and(|t| t.session.warp().is_some()), "in Warp mode");
    h.run_steps(4);
    let warp = rgb_at(&mut h, 135.5, 120.5);
    assert!(is_blue(warp), "Warp shows the picture as the Perspective left it until the warp is pulled: {warp:?}");
}

#[test]
fn warp_after_perspective_keeps_the_picture_on_the_gpu_canvas() {
    warp_after_perspective_keeps_the_picture(true);
}

#[test]
fn warp_after_perspective_keeps_the_picture_on_the_cpu_canvas() {
    warp_after_perspective_keeps_the_picture(false);
}

/// Right-click over the warp grid in Warp mode opens the transform menu in its Warp form, as the reference does
/// (#12493): the box modes, Warp with the Split Warp entries, and the rest; Scale picked there leaves Warp mode for the
/// box, as the Edit menu's Scale does (the warp held under it).
fn right_click_in_warp_opens_the_transform_menu(gpu_canvas: bool) {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness(gpu_canvas) else { return };
    setup(&mut h);
    control(&mut h, "ui.menu.invoke", json!({"id": "edit.transform.warp"}));
    assert!(h.state().ui.transform.as_ref().is_some_and(|t| t.session.warp().is_some()), "in Warp mode");
    let rows = ["Free Transform", "Scale", "Perspective", "Split Warp Crosswise", "Remove Warp Split", "Puppet Warp", "Flip Vertical"];
    let before: Vec<usize> = rows.iter().map(|row| h.query_all_by_label(row).count()).collect();
    let at = screen(&h, 150.0, 150.0);
    pointer_move(&mut h, at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(4);
    for (row, before) in rows.iter().zip(before) {
        assert!(h.query_all_by_label(row).count() > before, "the Warp menu lists {row}");
    }
    h.query_all_by_label("Scale").last().expect("the menu's Scale row").click();
    h.run_steps(4);
    let t = h.state().ui.transform.as_ref().expect("the transform stays open");
    assert!(t.session.warp().is_none(), "Scale picked in the Warp menu leaves Warp mode for the box");
}

#[test]
fn right_click_in_warp_opens_the_transform_menu_on_the_gpu_canvas() {
    right_click_in_warp_opens_the_transform_menu(true);
}

#[test]
fn right_click_in_warp_opens_the_transform_menu_on_the_cpu_canvas() {
    right_click_in_warp_opens_the_transform_menu(false);
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
