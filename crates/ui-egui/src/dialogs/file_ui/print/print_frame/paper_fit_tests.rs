use super::*;
use crate::PhotocraftApp;
use egui::{pos2, vec2};

fn fixture() -> (PhotocraftApp, Map<String, Value>) {
    photocraft_engine::print_cmds::disable_printing();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    let f = json!({"paper":"a4","orientation":"portrait","scale":37.123456,"center":false,"left":0.75,"top":1.25,"dryRun":true,"send":false})
        .as_object()
        .unwrap()
        .clone();
    (app, f)
}
fn resolved(app: &PhotocraftApp, f: &Map<String, Value>) -> PrintLayout {
    layout(&app.session.active().unwrap().doc, &params(f), "file.print").unwrap()
}
fn fit(paper: (f64, f64), viewport: Rect) -> (Rect, f32) {
    super::super::print_preview_page(paper, viewport).unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 0.002, "{a} != {b}");
}

#[test]
fn entire_paper_is_padded_centred_and_uniform_for_papers_orientations_and_viewports() {
    let (app, mut f) = fixture();
    for paper in ["a4", "letter"] {
        for orientation in ["portrait", "landscape"] {
            f.insert("paper".into(), json!(paper));
            f.insert("orientation".into(), json!(orientation));
            let l = resolved(&app, &f);
            for size in [vec2(330.0, 400.0), vec2(240.0, 180.0), vec2(80.0, 500.0), vec2(600.0, 120.0), vec2(12.0, 10.0)] {
                let viewport = Rect::from_min_size(pos2(37.0, 69.0), size);
                let (page, k) = fit(l.paper, viewport);
                let padding = 16.0_f32.min(size.min_elem() / 4.0);
                assert!(viewport.shrink(padding - 0.001).contains_rect(page));
                assert!((page.center() - viewport.center()).length() < 0.001);
                near(f64::from(page.width() / page.height()), l.paper.0 / l.paper.1);
                near(f64::from(page.width() / k), l.paper.0);
                near(f64::from(page.height() / k), l.paper.1);
                assert_eq!(fit(l.paper, viewport), (page, k), "fit is stateless and deterministic");
            }
        }
    }
}

#[test]
fn invalid_or_fully_clipped_viewport_has_no_display_transform() {
    let viewport = Rect::from_min_size(Pos2::ZERO, vec2(330.0, 400.0));
    for paper in [(0.0, 792.0), (-1.0, 792.0), (612.0, f64::NAN), (f64::INFINITY, 792.0)] {
        assert!(super::super::print_preview_page(paper, viewport).is_none());
    }
    for r in [Rect::NOTHING, Rect::from_min_size(Pos2::ZERO, Vec2::ZERO), Rect::from_min_max(pos2(10.0, 10.0), Pos2::ZERO)] {
        assert!(super::super::print_preview_page((612.0, 792.0), r).is_none());
    }
}

/// Inspect the actual paper draw, rather than assuming the nominal 330x400 allocation is
/// wholly visible. This reproduced the previous clipped-page failure on small windows.
fn draw(app: &mut PhotocraftApp, f: &mut Map<String, Value>, ctx: &egui::Context, screen: Vec2, nominal: Rect) -> (Rect, Rect) {
    let mut viewport = Rect::NOTHING;
    let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), ..Default::default() }, |ui| {
        ui.scope_builder(egui::UiBuilder::new().max_rect(nominal), |ui| {
            viewport = Rect::from_min_size(nominal.min, vec2(330.0, 400.0)).intersect(ui.clip_rect()).intersect(ui.ctx().content_rect());
            super::super::print_body(app, ui, f, 42);
        });
    });
    output.textures_delta.clear(); // headless shape inspection intentionally does not upload textures
    let page = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill == egui::Color32::WHITE && r.rect.width() > 20.0 && r.rect.height() > 20.0 => Some(r.rect),
            _ => None,
        })
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("paper rectangle drawn");
    (page, viewport)
}

#[test]
fn opening_clipped_preview_centres_whole_sheet_in_visible_region() {
    let (mut app, mut f) = fixture();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    for origin in [pos2(30.0, 100.0), pos2(-40.0, -60.0)] {
        let nominal = Rect::from_min_size(origin, vec2(720.0, 650.0));
        let (page, viewport) = draw(&mut app, &mut f, &ctx, vec2(640.0, 300.0), nominal);
        let expected = fit(resolved(&app, &f).paper, viewport).0;
        assert!((page.min - expected.min).length() < 0.002 && (page.max - expected.max).length() < 0.002);
        assert!(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 300.0)).contains_rect(page));
        assert!((page.center() - viewport.center()).length() < 0.002);
    }
}

#[test]
fn viewport_resize_and_paper_changes_refit_without_jitter_or_physical_layout_edits() {
    let (mut app, mut f) = fixture();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let doc = app.session.active().unwrap().doc.clone();
    let nominal = Rect::from_min_size(pos2(30.0, 100.0), vec2(720.0, 650.0));
    let physical = ["scale", "left", "top", "center"].map(|k| f[k].clone());
    for paper in ["a4", "letter"] {
        for orientation in ["portrait", "landscape"] {
            f.insert("paper".into(), json!(paper));
            f.insert("orientation".into(), json!(orientation));
            let start = resolved(&app, &f);
            let big = draw(&mut app, &mut f, &ctx, vec2(1100.0, 900.0), nominal).0;
            let small = draw(&mut app, &mut f, &ctx, vec2(640.0, 300.0), nominal).0;
            assert!(small.height() < big.height());
            let restored = draw(&mut app, &mut f, &ctx, vec2(1100.0, 900.0), nominal).0;
            assert_eq!(big, restored);
            for _ in 0..4 {
                assert_eq!(draw(&mut app, &mut f, &ctx, vec2(1100.0, 900.0), nominal).0, restored);
            }
            assert_eq!(resolved(&app, &f), start);
            assert_eq!(physical, ["scale", "left", "top", "center"].map(|k| f[k].clone()));
        }
    }
    assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
}

#[test]
fn refit_preserves_coordinate_conversion_dragging_and_pointer_anchored_print_scaling() {
    let (app, mut f) = fixture();
    for paper in ["a4", "letter"] {
        for orientation in ["portrait", "landscape"] {
            for mirrored in [false, true] {
                f.insert("paper".into(), json!(paper));
                f.insert("orientation".into(), json!(orientation));
                f.insert("emulsionDown".into(), json!(mirrored));
                let l = resolved(&app, &f);
                let mut wheel_result = None;
                for size in [vec2(330.0, 400.0), vec2(200.0, 130.0), vec2(180.0, 300.0)] {
                    let viewport = Rect::from_min_size(pos2(20.0, 30.0), size);
                    let (page, k) = fit(l.paper, viewport);
                    let img = print_preview_image(l, page, k);
                    let x = if mirrored { l.paper.0 - l.rect.0 - l.rect.2 } else { l.rect.0 };
                    near(f64::from((img.left() - page.left()) / k), x);
                    near(f64::from((img.top() - page.top()) / k), l.paper.1 - l.rect.1 - l.rect.3);
                    let mut moved = f.clone();
                    assert!(drag_print_position(&mut moved, l, vec2(9.0, -12.0) * k, k));
                    let after = print_preview_image(resolved(&app, &moved), page, k);
                    assert!((after.min - img.min - vec2(9.0, -12.0) * k).length() < 0.002);
                    let at = img.lerp_inside(vec2(0.3, 0.7));
                    let mut changed = f.clone();
                    assert!(wheel_scale(&mut changed, l, img, at, 40.0));
                    let updated = resolved(&app, &changed);
                    assert!((print_preview_image(updated, page, k).lerp_inside(vec2(0.3, 0.7)) - at).length() < 0.002);
                    if let Some(previous) = wheel_result {
                        let previous: PrintLayout = previous;
                        for (a, b) in [updated.rect.0, updated.rect.1, updated.rect.2, updated.rect.3, updated.scale].into_iter().zip([
                            previous.rect.0,
                            previous.rect.1,
                            previous.rect.2,
                            previous.rect.3,
                            previous.scale,
                        ]) {
                            near(a, b);
                        }
                    } else {
                        wheel_result = Some(updated);
                    }
                }
            }
        }
    }
}

#[test]
fn preview_open_and_viewport_refit_leave_pdf_and_print_parameters_unchanged() {
    let (mut app, mut f) = fixture();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paper-fit.pdf");
    let mut original = params(&f);
    original["output"] = json!(path.to_string_lossy());
    app.session.execute("file.print", original).unwrap();
    let before = std::fs::read(&path).unwrap();
    let l = resolved(&app, &f);
    for screen in [vec2(1100.0, 900.0), vec2(640.0, 300.0)] {
        draw(&mut app, &mut f, &ctx, screen, Rect::from_min_size(pos2(30.0, 100.0), vec2(720.0, 650.0)));
        let mut p = params(&f);
        p["output"] = json!(path.to_string_lossy());
        let result = app.session.execute("file.print", p).unwrap();
        assert_eq!(result["sent"], false);
        assert_eq!(before, std::fs::read(&path).unwrap());
        assert_eq!(resolved(&app, &f), l);
    }
}
