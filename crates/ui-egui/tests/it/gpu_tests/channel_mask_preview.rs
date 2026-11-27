//! Regressions in #4648's scalar preview: ordinary mask targeting must not require a separate
//! mask view, and displaying the preview must retain the canvas's full-resolution pixel sampling.
//! Original synthetic artwork, actual thumbnail clicks, Ctrl+T and corner-handle drags.

use craft::behavior::kurbo::Point;
use craft::behavior::{CanvasView, EditEvent, Mixed};
use craft::draw::ui::transform::TransformEdit;
use craft::layers::ui::tree::{LayerInfo, LayerKind, LayerPart, MaskInfo};
use egui::{Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::PixelFormat;
use photocraft_doc::LayerMask;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use photocraft_ui_egui::PhotocraftApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

fn harness(linked: bool, zoom: f32) -> Option<Harness> {
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 850.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
            app.background_jobs = false;
            app
        })
    });
    match built {
        Ok(mut h) => {
            // Fixture failures must fail the test, not masquerade as an absent adapter.
            let s = &mut h.state_mut().session;
            s.execute("file.new", json!({"name":"Unlinked mask preview", "width":320, "height":240, "background":"white"})).unwrap();
            s.execute("layer.new.layer", json!({"name":"Masked pattern"})).unwrap();
            s.edit("synthetic mask fixture", |doc, active| {
                let layer = doc.layer_mut(active.unwrap()).unwrap();
                for y in 0..240 {
                    for x in 0..320 {
                        let colors = [[0.08, 0.35, 0.85, 1.0], [0.95, 0.45, 0.10, 1.0], [0.15, 0.75, 0.45, 1.0]];
                        layer.surface_mut().unwrap().write_pixel(x, y, &colors[((x / 16 + y / 16) % 3) as usize]);
                    }
                }
                let mut mask = LayerMask::reveal_all();
                mask.linked = linked;
                mask.surface = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
                mask.surface.fill_rect(Rect::new(40, 30, 230, 200), &[1.0]);
                mask.surface.fill_rect(Rect::new(40, 30, 100, 85), &[0.0]);
                mask.surface.fill_rect(Rect::new(150, 100, 190, 145), &[0.0]);
                layer.mask = Some(mask);
                Ok(())
            })
            .unwrap();
            let app = h.state_mut();
            app.sync_views();
            app.ui.tool = photocraft_ui_egui::state::Tool::Move;
            app.ui.views[0].zoom = zoom;
            app.ui.views[0].center = [160.0, 120.0];
            app.ui.views[0].pending.clear();
            h.run_steps(8);
            Some(h)
        }
        Err(_) => {
            eprintln!("skipping mask-preview capture: no offscreen GPU adapter");
            None
        }
    }
}

fn pointer(h: &mut Harness, pos: Pos2, pressed: Option<bool>) {
    h.event(egui::Event::PointerMoved(pos));
    if let Some(pressed) = pressed {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(2);
}

struct Shot {
    width: u32,
    pixels: Vec<u8>,
    view: CanvasView,
}

impl Shot {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let p = self.view.to_screen(Point::new(x as f64 + 0.5, y as f64 + 0.5));
        let i = (p.y.floor() as usize * self.width as usize + p.x.floor() as usize) * 4;
        self.pixels[i..i + 4].try_into().unwrap()
    }
}

fn capture(h: &mut Harness, name: &str) -> Shot {
    h.run_steps(8);
    let view = photocraft_ui_egui::canvas::active_view(h.state()).unwrap();
    let image = h.render().unwrap();
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_MASK_TRANSFORM_EVIDENCE_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        image.save(dir.join(format!("{name}.png"))).unwrap();
    }
    Shot { width: image.width(), pixels: image.into_raw(), view }
}

#[test]
fn ordinary_mask_thumbnail_scaling_previews_and_commits_without_softening() {
    let _gpu = photocraft_testkit::gpu_lock();
    for linked in [false, true] {
        for zoom in [1.0, 2.0] {
            let Some(mut h) = harness(linked, zoom) else { return };
            let prefix = format!("mask-linked-{linked}-zoom-{zoom}");
            let id = h.state().session.active().unwrap().active_layer.unwrap();
            let mut info = LayerInfo::new("Masked pattern", LayerKind::Pixel);
            info.masks.push(MaskInfo::new(LayerPart::LayerMask, true, linked));
            let node = h.get_by_role_and_label(egui::accesskit::Role::TreeItem, "Masked pattern").accesskit_node().locate().0;
            let thumb = craft::layers::testing::thumb_rect(&h.ctx, node, &info, LayerPart::LayerMask).unwrap().center();
            pointer(&mut h, thumb, Some(true));
            pointer(&mut h, thumb, Some(false));
            assert!(h.state().ui.mask_target);
            assert!(h.state().session.active().unwrap().channel_view.layer_mask.is_none());
            let before = h.state().session.active().unwrap().doc.clone();
            let eyes = h.state().session.active().unwrap().channel_view.clone();
            let first = capture(&mut h, &format!("{prefix}-before"));
            h.key_press_modifiers(Modifiers::COMMAND, egui::Key::T);
            h.run_steps(4);
            let t = h.state().ui.transform.clone().expect("a normally targeted mask must open without a separate mask view");
            assert_eq!(t.target, Some(json!("mask")));
            assert_eq!(h.state().session.active().unwrap().channel_view, eyes);
            let view = photocraft_ui_egui::canvas::active_view(h.state()).unwrap();
            let start = view.to_screen(Point::new(t.quad_array()[2][0], t.quad_array()[2][1]));
            pointer(&mut h, start, Some(true));
            for step in 1..=4 {
                let p = view.to_screen(Point::new(t.quad_array()[2][0] + step as f64 * 5.0, t.quad_array()[2][1] + step as f64 * 3.0));
                pointer(&mut h, p, None);
            }
            let middle = capture(&mut h, &format!("{prefix}-mid-drag"));
            assert!(h.state().ui.transform.is_some());
            assert_eq!(h.state().session.active().unwrap().doc, before);
            // This point starts inside the white hole and is revealed by the larger mask.
            // It is away from the box and pivot, so moving controls cannot satisfy the check.
            assert_eq!(first.at(155, 110), [255; 4]);
            assert_ne!(middle.at(155, 110), first.at(155, 110), "the mask itself updates before release");
            let changed = (8..232).flat_map(|y| (8..312).map(move |x| (x, y))).filter(|&(x, y)| first.at(x, y) != middle.at(x, y)).count();
            assert!(changed > 500, "the live mask must visibly change during the drag");
            let end = view.to_screen(Point::new(t.quad_array()[2][0] + 40.0, t.quad_array()[2][1] + 24.0));
            pointer(&mut h, end, None);
            pointer(&mut h, end, Some(false));
            pointer(&mut h, Pos2::new(25.0, 110.0), None);
            let scaled = h.state().ui.transform.clone().unwrap();
            let live = capture(&mut h, &format!("{prefix}-active"));
            assert_eq!(h.state().session.active().unwrap().doc, before);
            h.key_press(egui::Key::Enter);
            h.run_steps(4);
            assert!(h.state().ui.transform.is_none());
            assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
            let committed = capture(&mut h, &format!("{prefix}-committed"));
            let after = h.state().session.active().unwrap().doc.clone();
            assert_eq!(after.layer(id).unwrap().surface(), before.layer(id).unwrap().surface(), "host samples remain stationary");
            assert_ne!(after.layer(id).unwrap().mask, before.layer(id).unwrap().mask);
            let [left, top] = scaled.quad_array()[0];
            let [right, bottom] = scaled.quad_array()[2];
            let mut compared = 0;
            for y in 8..232 {
                for x in 8..312 {
                    let p = [x as f64 + 0.5, y as f64 + 0.5];
                    // Only the transform's controls are absent after Enter. Keep checker and
                    // internal mask edges in the comparison: they exposed the filtering bug.
                    if [p[0] - left, p[0] - right, p[1] - top, p[1] - bottom].iter().any(|d| d.abs() < 10.0)
                        || (p[0] - scaled.session.pivot().x).abs() < 10.0 && (p[1] - scaled.session.pivot().y).abs() < 10.0
                    {
                        continue;
                    }
                    assert_eq!(live.at(x, y), committed.at(x, y), "linked={linked} zoom={zoom} at {x},{y}");
                    compared += 1;
                }
            }
            assert!(compared > 20_000);
            assert_eq!(h.state().session.active().unwrap().channel_view, eyes);
            assert!(h.state_mut().session.undo());
            assert_eq!(h.state().session.active().unwrap().doc, before);
            h.key_press_modifiers(Modifiers::COMMAND, egui::Key::T);
            h.run_steps(4);
            let t = h.state_mut().ui.transform.as_mut().unwrap();
            let box_before = *t.session.state();
            let mut after = box_before;
            after.quad[2].x += 10.0;
            t.session.apply(TransformEdit::Box(EditEvent::set(Mixed::Same(box_before), after, "Edit box".into(), None)), 0.0);
            h.run_steps(2);
            h.key_press(egui::Key::Escape);
            h.run_steps(4);
            assert!(h.state().ui.transform.is_none());
            assert_eq!(h.state().session.active().unwrap().doc, before);
        }
    }
}
