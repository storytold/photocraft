use super::*;
use crate::PhotocraftApp;
use egui::{pos2, vec2};
use egui_kittest::Harness;

fn fixture() -> (PhotocraftApp, Map<String, Value>) {
    photocraft_engine::print_cmds::disable_printing();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400,"resolution":72})).unwrap();
    let fields = json!({"paper":"a4","center":true,"scale":40.123456,"dryRun":true,"send":false}).as_object().unwrap().clone();
    (app, fields)
}
fn resolved(app: &PhotocraftApp, f: &Map<String, Value>) -> PrintLayout {
    layout(&app.session.active().unwrap().doc, &params(f), "file.print").unwrap()
}
fn page(l: PrintLayout, k: f32) -> Rect {
    Rect::from_min_size(pos2(17.0, 23.0), vec2(l.paper.0 as f32 * k, l.paper.1 as f32 * k))
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 0.002, "{a} != {b}");
}

#[test]
fn full_canvas_bounds_include_blank_white_and_transparent_margins() {
    let (mut app, f) = fixture();
    for color in ["#ffffff", "#00000000"] {
        app.run("edit.fill", json!({"color":color})).unwrap();
        let l = resolved(&app, &f);
        let doc = &app.session.active().unwrap().doc;
        let k = 0.45;
        let r = print_preview_image(l, page(l, k), k);
        near(f64::from(r.width() / k), f64::from(doc.size.width) * 72.0 / f64::from(doc.resolution_dpi) * l.scale);
        near(f64::from(r.height() / k), l.rect.3);
        assert_eq!(hit(r, r.center()), Some(Target::Move));
        assert_eq!(handles(r).len(), 8);
    }
}

#[test]
fn handle_hit_testing_prefers_nearest_handle_and_rejects_outside() {
    let r = Rect::from_min_size(pos2(30.0, 40.0), vec2(200.0, 120.0));
    for (target, p) in handles(r) {
        assert_eq!(hit(r, p), Some(target));
        assert_eq!(hit(r, p + vec2(2.0, 2.0)), Some(target));
    }
    assert_eq!(hit(r, r.center()), Some(Target::Move));
    assert_eq!(hit(r, r.min - vec2(8.0, 8.0)), None);
    let small = Rect::from_center_size(r.center(), vec2(8.0, 6.0));
    for (target, p) in handles(small) {
        assert_eq!(hit(small, p), Some(target));
    }
}

#[test]
fn all_eight_handles_preserve_aspect_and_opposite_anchor_across_paper_mirror_and_zoom() {
    let (app, base) = fixture();
    for paper_name in ["a4", "letter"] {
        for orientation in ["portrait", "landscape"] {
            for mirror in [false, true] {
                for fit in [false, true] {
                    let mut f = base.clone();
                    f.insert("paper".into(), json!(paper_name));
                    f.insert("orientation".into(), json!(orientation));
                    f.insert("emulsionDown".into(), json!(mirror));
                    f.insert("scaleToFit".into(), json!(fit));
                    let start = resolved(&app, &f);
                    for k in [0.25, 0.5, 1.0, 2.0] {
                        let pg = page(start, k);
                        let old = print_preview_image(start, pg, k);
                        for (target, _) in handles(old) {
                            let (sx, sy) = match target {
                                Target::Resize(x, y) => (x, y),
                                Target::Move => (0, 0),
                            };
                            assert_ne!((sx, sy), (0, 0), "handles must all resize");
                            let delta = vec2(f32::from(sx) * old.width() * 0.2, f32::from(sy) * old.height() * 0.2);
                            let mut changed = f.clone();
                            assert!(resize(&mut changed, start, delta, k, target));
                            let l = resolved(&app, &changed);
                            near(l.scale, start.scale * 1.2);
                            near(l.rect.2 / l.rect.3, start.rect.2 / start.rect.3);
                            let new = print_preview_image(l, pg, k);
                            let anchor = |r: Rect| pos2(r.center().x - f32::from(sx) * r.width() / 2.0, r.center().y - f32::from(sy) * r.height() / 2.0);
                            assert!((anchor(old) - anchor(new)).length() < 0.002);
                            assert_eq!(changed["center"], false);
                            assert_eq!(changed["scaleToFit"], false);
                            near(changed["scale"].as_f64().unwrap(), l.scale * 100.0);
                            near(changed["top"].as_f64().unwrap() * 72.0, l.paper.1 - l.rect.1 - l.rect.3);
                            near(changed["left"].as_f64().unwrap() * 72.0, l.rect.0);
                            let once = changed.clone();
                            assert!(resize(&mut changed, start, delta, k, target));
                            assert_eq!(once, changed, "total displacement must not accumulate");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn resize_rejects_invalid_input_and_never_reflects_at_anchor() {
    let (app, mut f) = fixture();
    let l = resolved(&app, &f);
    let original = f.clone();
    for k in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(!resize(&mut f, l, vec2(10.0, 10.0), k, Target::Resize(1, 1)));
        assert_eq!(f, original);
    }
    assert!(!resize(&mut f, l, vec2(f32::NAN, 0.0), 1.0, Target::Resize(1, 1)));
    assert_eq!(f, original);
    assert!(resize(&mut f, l, vec2(-1.0e6, -1.0e6), 1.0, Target::Resize(1, 1)));
    near(resolved(&app, &f).scale, 0.01);
    assert!(resize(&mut f, l, vec2(1.0e6, 1.0e6), 1.0, Target::Resize(1, 1)));
    near(resolved(&app, &f).scale, 10.0);
}

fn harness() -> Harness<'static, (PhotocraftApp, Map<String, Value>)> {
    let mut h = Harness::builder().with_size(vec2(1050.0, 900.0)).build_ui_state(
        |ui, state: &mut (PhotocraftApp, Map<String, Value>)| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::widgets::field_wheel::begin_frame(ui.ctx());
            super::super::print_body(&mut state.0, ui, &mut state.1, 42);
            let after = ui.input(|i| (i.events.iter().filter(|e| matches!(e, egui::Event::MouseWheel { .. })).count(), i.smooth_scroll_delta));
            ui.data_mut(|d| d.insert_temp(egui::Id::unique("print-wheel-after"), after));
        },
        fixture(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}
fn preview_rect(h: &Harness<'_, (PhotocraftApp, Map<String, Value>)>) -> Rect {
    use egui_kittest::kittest::Queryable;
    h.get_by_label("Print preview image").rect().shrink(HIT_RADIUS)
}

fn pointer(h: &mut Harness<'_, (PhotocraftApp, Map<String, Value>)>, p: Pos2, button: Option<bool>) {
    h.event(egui::Event::PointerMoved(p));
    if let Some(pressed) = button {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(2);
}

#[test]
fn pointer_resize_then_move_updates_existing_controls_without_document_changes() {
    let mut h = harness();
    let doc = h.state().0.session.active().unwrap().doc.clone();
    let revision = h.state().0.session.active().unwrap().revision;
    let start = resolved(&h.state().0, &h.state().1);
    let r = preview_rect(&h);
    let p = r.right_bottom();
    pointer(&mut h, p, Some(true));
    pointer(&mut h, p + vec2(25.0, 20.0), None);
    let during = resolved(&h.state().0, &h.state().1);
    assert!(during.scale > start.scale, "handle should resize, not translate");
    pointer(&mut h, p + vec2(30.0, 25.0), Some(false));
    let resized = resolved(&h.state().0, &h.state().1);
    assert!(resized.scale > during.scale);
    let p = preview_rect(&h).center();
    pointer(&mut h, p, Some(true));
    pointer(&mut h, p + vec2(20.0, -10.0), None);
    pointer(&mut h, p + vec2(20.0, -10.0), Some(false));
    let moved = resolved(&h.state().0, &h.state().1);
    near(moved.scale, resized.scale);
    assert_ne!(moved.rect.0, resized.rect.0);
    assert_ne!(moved.rect.1, resized.rect.1);
    h.run_steps(5);
    assert_eq!(resolved(&h.state().0, &h.state().1), moved, "numeric widgets must not round mouse state each frame");
    assert!(std::sync::Arc::ptr_eq(&doc, &h.state().0.session.active().unwrap().doc));
    assert_eq!(h.state().0.session.active().unwrap().revision, revision);
}

#[test]
fn numeric_fields_update_frame_and_remain_stable_without_pointer_input() {
    let mut h = harness();
    let before = preview_rect(&h);
    let f = &mut h.state_mut().1;
    f.insert("center".into(), json!(false));
    f.insert("scale".into(), json!(75.123456));
    f.insert("top".into(), json!(0.8));
    f.insert("left".into(), json!(0.4));
    h.run_steps(4);
    let after = preview_rect(&h);
    assert_ne!(before, after);
    let l = resolved(&h.state().0, &h.state().1);
    near(l.rect.0, 0.4 * 72.0);
    near(l.paper.1 - l.rect.1 - l.rect.3, 0.8 * 72.0);
    assert_eq!(h.state().1["scale"], json!(75.123456));
    near(f64::from(after.width() / before.width()), 75.123456 / 40.123456);
}

#[test]
fn overlays_are_egui_only_and_do_not_change_pdf_or_document() {
    let mut h = harness();
    let doc = h.state().0.session.active().unwrap().doc.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preview-frame.pdf");
    let mut p = params(&h.state().1);
    p["output"] = json!(path.to_string_lossy());
    let result = h.state_mut().0.session.execute("file.print", p.clone()).unwrap();
    assert_eq!(result["sent"], false);
    let before = std::fs::read(&path).unwrap();
    let f = h.state().1.clone();
    h.run_steps(3);
    assert_eq!(f, h.state().1, "drawing the frame must not add output fields");
    h.state_mut().0.session.execute("file.print", p).unwrap();
    assert_eq!(before, std::fs::read(&path).unwrap());
    assert!(std::sync::Arc::ptr_eq(&doc, &h.state().0.session.active().unwrap().doc));
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_PRINT_FRAME_SNAPSHOT_DIR") {
        h.render().unwrap().save(std::path::PathBuf::from(dir).join("print-frame-preview.png")).unwrap();
    }
}

#[test]
fn numeric_width_height_scale_and_offsets_edit_the_same_layout_as_handles() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness();
    h.state_mut().1.insert("center".into(), json!(false));
    h.run_steps(3);
    assert_eq!(h.get_all_by_role(egui::Role::SpinButton).count(), 6);
    let edit = |h: &mut Harness<'_, (PhotocraftApp, Map<String, Value>)>, index, text: &str| {
        h.get_all_by_role(egui::Role::SpinButton).nth(index).unwrap().click();
        h.run();
        // Paste a complete numeric edit through the existing field parser.
        h.event(egui::Event::Text(text.to_string()));
        h.run();
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    };
    for (index, text, expected_width) in [(5, "9.25", 9.25 * 72.0 / 2.54), (4, "5.0", 5.0 * 72.0 / 2.54 * 1.5)] {
        edit(&mut h, index, text);
        let l = resolved(&h.state().0, &h.state().1);
        // Existing numeric width/height edits round percentage to two decimals.
        assert!((l.rect.2 - expected_width).abs() < 0.04, "index {index}: actual {} expected {expected_width}; fields {:?}", l.rect.2, h.state().1);
        near(l.rect.2 / l.rect.3, 1.5);
        near(h.state().1["scale"].as_f64().unwrap(), l.scale * 100.0);
    }
    edit(&mut h, 3, "64.5");
    near(resolved(&h.state().0, &h.state().1).scale, 0.645);
    edit(&mut h, 1, "1.25");
    edit(&mut h, 2, "2.5");
    let l = resolved(&h.state().0, &h.state().1);
    near(l.rect.0, 2.5 * 72.0 / 2.54);
    near(l.paper.1 - l.rect.1 - l.rect.3, 1.25 * 72.0 / 2.54);
    let r = preview_rect(&h);
    near(f64::from(r.width() / r.height()), 1.5);
}

#[test]
fn live_side_resize_preserves_aspect_and_layout_edit_cancels_capture() {
    let mut h = harness();
    let r = preview_rect(&h);
    let start = resolved(&h.state().0, &h.state().1);
    let p = r.right_center();
    pointer(&mut h, p, Some(true));
    pointer(&mut h, p + vec2(20.0, 12.0), None);
    let changed = resolved(&h.state().0, &h.state().1);
    assert!(changed.scale > start.scale);
    near(changed.rect.2 / changed.rect.3, start.rect.2 / start.rect.3);
    h.state_mut().1.insert("scale".into(), json!(30.0));
    h.run_steps(2);
    pointer(&mut h, p + vec2(35.0, 12.0), None);
    pointer(&mut h, p + vec2(40.0, 12.0), Some(false));
    near(resolved(&h.state().0, &h.state().1).scale, 0.3);
}

#[test]
fn wheel_anchors_full_canvas_across_papers_orientations_mirroring_fit_and_preview_scales() {
    let (app, base) = fixture();
    for paper_name in ["a4", "letter"] {
        for orientation in ["portrait", "landscape"] {
            for mirror in [false, true] {
                for fit in [false, true] {
                    let mut f = base.clone();
                    f.insert("paper".into(), json!(paper_name));
                    f.insert("orientation".into(), json!(orientation));
                    f.insert("emulsionDown".into(), json!(mirror));
                    f.insert("scaleToFit".into(), json!(fit));
                    let start = resolved(&app, &f);
                    for k in [0.25, 0.5, 1.0, 2.0] {
                        let pg = page(start, k);
                        let img = print_preview_image(start, pg, k);
                        for fraction in [vec2(0.0, 0.0), vec2(0.3, 0.7), vec2(1.0, 1.0)] {
                            let at = img.lerp_inside(fraction);
                            for points in [-40.0, 40.0] {
                                let mut changed = f.clone();
                                assert!(wheel_scale(&mut changed, start, img, at, points));
                                let l = resolved(&app, &changed);
                                assert_eq!(l.scale > start.scale, points > 0.0);
                                near(l.scale, start.scale * (points * std::f64::consts::LN_2 / 300.0).exp());
                                near(l.rect.2 / l.rect.3, start.rect.2 / start.rect.3);
                                let updated = print_preview_image(l, pg, k);
                                assert!((updated.lerp_inside(fraction) - at).length() < 0.002);
                                near(changed["scale"].as_f64().unwrap(), l.scale * 100.0);
                                near(changed["left"].as_f64().unwrap() * 72.0, l.rect.0);
                                near(changed["top"].as_f64().unwrap() * 72.0, l.paper.1 - l.rect.1 - l.rect.3);
                                assert_eq!(changed["scaleToFit"], false);
                                assert_eq!(changed["center"], false);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn wheel_limits_invalid_input_and_display_rounding_never_corrupt_geometry() {
    let (app, mut f) = fixture();
    let start = resolved(&app, &f);
    let image = print_preview_image(start, page(start, 0.5), 0.5);
    for points in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0] {
        let before = f.clone();
        assert!(!wheel_scale(&mut f, start, image, image.center(), points));
        assert_eq!(f, before);
    }
    for (points, expected) in [(-f64::MAX, 0.01), (f64::MAX, 10.0)] {
        assert!(wheel_scale(&mut f, start, image, image.center(), points));
        let limit = resolved(&app, &f);
        near(limit.scale, expected);
        let before = f.clone();
        assert!(wheel_scale(&mut f, limit, print_preview_image(limit, page(limit, 0.5), 0.5), image.center(), points));
        assert_eq!(f, before, "wheel at a limit must not drift placement");
    }
}

fn wheel_event(unit: egui::MouseWheelUnit, delta: Vec2, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::MouseWheel { source: egui::MouseWheelSource::Unknown, unit, delta, phase: egui::TouchPhase::Move, modifiers }
}

#[test]
fn wheel_normalizes_units_and_leaves_modified_horizontal_or_invalid_input_alone() {
    for (unit, y, expected) in [(egui::MouseWheelUnit::Point, 40.0, 40.0), (egui::MouseWheelUnit::Line, 1.0, 40.0), (egui::MouseWheelUnit::Page, 0.1, 40.0)] {
        near(wheel_points(&wheel_event(unit, vec2(0.0, y), egui::Modifiers::NONE), 40.0, 400.0).unwrap(), expected);
    }
    for delta in [Vec2::ZERO, vec2(30.0, 0.0), vec2(0.0, f32::NAN)] {
        assert!(wheel_points(&wheel_event(egui::MouseWheelUnit::Point, delta, egui::Modifiers::NONE), 40.0, 400.0).is_none());
    }
    for mods in [egui::Modifiers::CTRL, egui::Modifiers::ALT, egui::Modifiers::SHIFT, egui::Modifiers::MAC_CMD] {
        assert!(wheel_points(&wheel_event(egui::MouseWheelUnit::Point, vec2(0.0, 40.0), mods), 40.0, 400.0).is_none());
    }
}

fn wheel(h: &mut Harness<'_, (PhotocraftApp, Map<String, Value>)>, at: Pos2, points: f32) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(wheel_event(egui::MouseWheelUnit::Point, vec2(0.0, points), egui::Modifiers::NONE));
    h.run_steps(1);
}

#[test]
fn wheel_ui_changes_print_size_and_fields_not_preview_zoom_or_document_and_consumes_input() {
    let mut h = harness();
    let doc = h.state().0.session.active().unwrap().doc.clone();
    let history = h.state().0.session.active().unwrap().history.past_len();
    let start = resolved(&h.state().0, &h.state().1);
    let r = preview_rect(&h);
    let at = r.lerp_inside(vec2(0.3, 0.6));
    wheel(&mut h, at, 40.0);
    let l = resolved(&h.state().0, &h.state().1);
    assert!(l.scale > start.scale);
    assert_eq!(l.paper, start.paper);
    let after: (usize, Vec2) = h.ctx.data(|d| d.get_temp(egui::Id::unique("print-wheel-after"))).unwrap();
    assert_eq!(after.0, 0);
    assert_eq!(after.1.y, 0.0);
    h.run_steps(1); // accessibility hit bounds catch up to the already-painted frame
    near(f64::from(preview_rect(&h).width() / r.width()), l.scale / start.scale);
    h.run_steps(8);
    assert_eq!(resolved(&h.state().0, &h.state().1), l, "egui smoothing tail must not scale twice");
    wheel(&mut h, at, -40.0);
    near(resolved(&h.state().0, &h.state().1).scale, start.scale);
    assert!(std::sync::Arc::ptr_eq(&doc, &h.state().0.session.active().unwrap().doc));
    assert_eq!(h.state().0.session.active().unwrap().history.past_len(), history);
}

#[test]
fn wheel_over_handles_works_and_wheel_elsewhere_remains_available() {
    let mut h = harness();
    let at = preview_rect(&h).right_top();
    wheel(&mut h, at, 40.0);
    let scaled = resolved(&h.state().0, &h.state().1);
    wheel(&mut h, pos2(20.0, 700.0), -40.0);
    assert_eq!(resolved(&h.state().0, &h.state().1), scaled);
    let after: (usize, Vec2) = h.ctx.data(|d| d.get_temp(egui::Id::unique("print-wheel-after"))).unwrap();
    assert_eq!(after.0, 1, "ordinary wheel outside preview remains available");
    assert_ne!(after.1.y, 0.0);
}

#[test]
fn wheel_over_canvas_reclaims_input_from_focused_numeric_field() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness();
    // The Copies field would otherwise capture wheel globally; it must not change here.
    h.get_all_by_role(egui::Role::SpinButton).next().unwrap().click();
    h.run_steps(2);
    assert!(h.ctx.memory(|m| m.focused()).is_some(), "numeric field must have keyboard focus");
    let copies = h.state().1["copies"].clone();
    let start = resolved(&h.state().0, &h.state().1);
    let at = preview_rect(&h).center();
    wheel(&mut h, at, 40.0);
    assert!(resolved(&h.state().0, &h.state().1).scale > start.scale);
    assert_eq!(h.state().1["copies"], copies);
}

#[test]
fn footer_follows_cm_mm_in_units_without_rounding_physical_fields_or_pdf() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness();
    let at = preview_rect(&h).center();
    wheel(&mut h, at, 40.0);
    let l = resolved(&h.state().0, &h.state().1);
    let physical = ["scale", "left", "top"].map(|key| h.state().1[key].clone());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wheel-preview.pdf");
    let mut first = None;
    for units in ["cm", "mm", "in"] {
        h.state_mut().1.insert("units".into(), json!(units));
        h.run_steps(3);
        let text = super::super::print_preview_footer(l, units);
        assert!(h.query_by_label(&text).is_some(), "footer: {text}");
        assert!(text.contains(&format!("{:.2} × {:.2} {units}", l.rect.2 / super::super::unit_pt(units), l.rect.3 / super::super::unit_pt(units))));
        assert_eq!(physical, ["scale", "left", "top"].map(|key| h.state().1[key].clone()));
        let mut p = params(&h.state().1);
        p["output"] = json!(path.to_string_lossy());
        let result = h.state_mut().0.session.execute("file.print", p).unwrap();
        assert_eq!(result["sent"], false);
        let rect = result["imageRect"].as_array().unwrap();
        for (actual, expected) in rect.iter().zip([l.rect.0, l.rect.1, l.rect.2, l.rect.3]) {
            near(actual.as_f64().unwrap(), expected);
        }
        let bytes = std::fs::read(&path).unwrap();
        if let Some(first) = &first {
            assert_eq!(first, &bytes, "display units cannot affect PDF output");
        } else {
            first = Some(bytes);
        }
    }
    let mut l = l;
    l.rect.2 = 4.97 * 72.0;
    l.rect.3 = 2.80 * 72.0;
    assert!(super::super::print_preview_footer(l, "cm").contains("12.62 × 7.11 cm"));
    assert!(super::super::print_preview_footer(l, "in").contains("4.97 × 2.80 in"));
}

#[test]
fn wheel_during_handle_drag_is_not_applied_or_consumed_by_preview() {
    let mut h = harness();
    let at = preview_rect(&h).right_bottom();
    pointer(&mut h, at, Some(true));
    pointer(&mut h, at + vec2(10.0, 10.0), None);
    let before = resolved(&h.state().0, &h.state().1);
    h.event(wheel_event(egui::MouseWheelUnit::Point, vec2(0.0, 40.0), egui::Modifiers::NONE));
    h.run_steps(1);
    assert_eq!(resolved(&h.state().0, &h.state().1), before);
    let after: (usize, Vec2) = h.ctx.data(|d| d.get_temp(egui::Id::unique("print-wheel-after"))).unwrap();
    assert_eq!(after.0, 1);
    pointer(&mut h, at + vec2(10.0, 10.0), Some(false));
}
