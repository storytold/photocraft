//! The second user review: real gesture outcomes, canonical targets and active-frame cost.
use super::tests::{compact_pair, harness, layout, press, release};
use super::*;
use egui::Event;
use egui_kittest::Harness;

#[test]
fn resized_flyout_remains_open_at_the_committed_size_on_both_axes() {
    for bottom in [false, true] {
        let mut h = harness(compact_pair());
        h.state_mut().run("window.panel.group", json!({"panel":"properties","operation":"peek"})).unwrap();
        h.run_steps(5);
        let before = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
        let from = if bottom { pos2(before.center().x, before.bottom() - 1.0) } else { pos2(before.right() - 1.0, before.center().y) };
        let to = from + if bottom { vec2(0.0, 40.0) } else { vec2(60.0, 0.0) };
        press(&mut h, from, to);
        release(&mut h, to);
        h.run_steps(5);
        assert_eq!(h.state().ui.docking_flyout.as_deref(), Some("properties"));
        let after = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
        assert!((after.width() - before.width() - if bottom { 0.0 } else { 60.0 }).abs() < 1.1);
        assert!((after.height() - before.height() - if bottom { 40.0 } else { 0.0 }).abs() < 1.1);
        let saved = crate::dock::snapshot(h.state());
        let mut restored = compact_pair();
        crate::dock::apply(&mut restored, &saved);
        assert_eq!(restored.ui.panel_presentation.flyouts, h.state().ui.panel_presentation.flyouts);
    }
}
#[test]
fn clickable_panel_chrome_uses_the_pointer_cursor() {
    let mut h = harness(compact_pair());
    for part in ["expand", "close", "icon"] {
        let rect = h.ctx.read_response(key(part).with("properties")).unwrap().rect;
        h.event(Event::PointerMoved(rect.center()));
        h.run_steps(3);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::PointingHand, "{part}");
    }
}

#[test]
#[ignore = "explicit active panel frame benchmark"]
fn benchmark_panel_hover_and_drag_frames() {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(1.25).build_eframe(|cc| super::followup_tests::full_app(&cc.egui_ctx));
    h.run_steps(64);
    let source = h.ctx.read_response(key("grip").with("properties")).unwrap().rect.center();
    let icon = h.ctx.read_response(key("icon").with("properties")).unwrap().rect.center();
    let arrow = h.ctx.read_response(key("expand").with("properties")).unwrap().rect.center();
    let mut result = serde_json::Map::new();
    for phase in ["idle", "hover", "drag"] {
        if phase == "drag" {
            press(&mut h, source, source + vec2(20.0, 20.0));
        }
        let mut samples = Vec::new();
        for i in 0..640 {
            let at = match phase {
                "hover" => {
                    if i % 2 == 0 {
                        icon
                    } else {
                        arrow
                    }
                }
                "drag" => source + vec2(50.0 + (i % 80) as f32 * 4.0, 50.0 + (i % 30) as f32),
                _ => pos2(600.0, 500.0),
            };
            let start = std::time::Instant::now();
            h.event(Event::PointerMoved(at));
            h.run_steps(1);
            if i >= 40 {
                samples.push(start.elapsed().as_secs_f64() * 1e6);
            }
        }
        samples.sort_by(f64::total_cmp);
        result.insert(phase.into(), json!({"p50_us":samples[300],"p95_us":samples[570],"p99_us":samples[594],"max_us":samples[599],"samples":600}));
        if phase == "drag" {
            release(&mut h, source + vec2(200.0, 100.0));
        }
    }
    println!("ACTIVE_PANEL_BENCH {}", Value::Object(result));
}

fn full_pair() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    for (panel, x) in [("color", 450), ("brushes", 850)] {
        app.run("window.panel.edge", json!({"panel":panel,"edge":"floating","individual":true,"collapsed":false,"x":x,"y":200,"width":320,"height":400}))
            .unwrap();
    }
    app
}
fn body_rect(h: &Harness<'static, PhotocraftApp>, id: &str) -> Rect {
    {
        let members = group_members(layout(h.state()), id);
        super::super::last_bodies(&h.ctx).into_iter().find(|(p, _)| members.contains(p)).unwrap().1
    }
}
fn tab_rect(h: &Harness<'static, PhotocraftApp>, id: &str) -> Rect {
    DockArea::<String>::hit_rects(&h.ctx, area_id()).into_iter().find(|(hit, _)| matches!(hit,Hit::Tab(p) if p==id)).unwrap().1
}
fn strip_rect(h: &Harness<'static, PhotocraftApp>, id: &str) -> Rect {
    strips(h.state(), &h.ctx, layout(h.state())).into_iter().find(|s| s.panels().any(|p| p == id)).unwrap().rect
}
fn click(h: &mut Harness<'static, PhotocraftApp>, id: egui::Id) {
    let p = h.ctx.read_response(id).unwrap().rect.center();
    h.event(Event::PointerMoved(p));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(5);
}
#[test]
fn top_preview_is_exactly_as_wide_as_the_canvas_it_splits() {
    let h = harness(full_pair());
    let canvas = body_rect(&h, super::super::CANVAS);
    let target = horizontal_landing(&h.ctx, pos2(canvas.center().x, canvas.top() + 1.0)).unwrap();
    assert_eq!(target.marker.x_range(), canvas.x_range());
    let mut commands = Vec::new();
    let feedback = resolve_drag(
        h.state(),
        &h.ctx,
        &DropRequest {
            panel: &"brushes".into(),
            pointer: pos2(canvas.center().x, canvas.top() + 1.0),
            targets: &[],
            floating_rect: Some(body_rect(&h, "brushes")),
            whole_group: true,
            released: false,
            workspace: h.ctx.content_rect(),
            screen: h.ctx.content_rect(),
        },
        &mut commands,
    )
    .unwrap();
    assert!(feedback.preview.is_none());
    assert_eq!(feedback.insertion.unwrap().x_range(), canvas.x_range());
}
#[test]
fn full_groups_only_offer_tab_slots_and_distinct_outer_split_edges() {
    let mut app = full_pair();
    app.run("window.panel.group", json!({"panel":"navigator","operation":"tab","anchor":"color"})).unwrap();
    let h = harness(app);
    let color = tab_rect(&h, "color");
    let nav = tab_rect(&h, "navigator");
    let body = body_rect(&h, "color");
    for (x, before) in [(color.left(), Some("color")), (color.right(), Some("navigator")), (nav.right(), None)] {
        for offset in [-3.0, 0.0, 3.0] {
            let (target, owns) = full_landing(&h.ctx, layout(h.state()), "brushes", true, pos2(x + offset, color.center().y), false);
            assert!(owns);
            let target = target.unwrap();
            assert!(matches!(target.target,Target::At{placement:Placement::Tab{before:ref b},..} if b.as_deref()==before));
            assert!((target.marker.width() - h.ctx.pixels_per_point().recip()).abs() < 0.01);
        }
    }
    for p in [color.center(), nav.center(), body.center(), pos2(body.center().x, body.top() + 2.0)] {
        let (target, _) = full_landing(&h.ctx, layout(h.state()), "brushes", true, p, false);
        assert!(target.is_none(), "extra tab-merging target at {p:?}");
    }
    for (p, zone) in [(pos2(body.center().x, color.top() - 2.0), Zone::Top), (pos2(body.center().x, body.bottom() - 2.0), Zone::Bottom)] {
        let target = full_landing(&h.ctx, layout(h.state()), "brushes", true, p, false).0.unwrap();
        assert!(matches!(target.target,Target::At{placement:Placement::Split(z),..} if z==zone));
        let y = if zone == Zone::Top { color.top() } else { body.bottom() };
        assert!((target.marker.center().y - y).abs() < 0.1);
    }
}
#[test]
fn mouse_inserts_before_between_and_after_tabs_and_reorders_its_own_group() {
    for slot in 0..3 {
        let mut app = full_pair();
        app.run("window.panel.group", json!({"panel":"navigator","operation":"tab","anchor":"color"})).unwrap();
        let mut h = harness(app);
        let a = tab_rect(&h, "color");
        let b = tab_rect(&h, "navigator");
        let to = pos2(
            match slot {
                0 => a.left() + 2.0,
                1 => a.right(),
                _ => b.right() + 2.0,
            },
            a.center().y,
        );
        let from = tab_rect(&h, "brushes").center();
        press(&mut h, from, to);
        release(&mut h, to);
        let mut expected = vec!["color".to_owned(), "navigator".to_owned()];
        expected.insert(slot, "brushes".into());
        assert_eq!(group_members(layout(h.state()), "color"), expected, "slot {slot}");
    }
    let mut app = full_pair();
    app.run("window.panel.group", json!({"panel":"navigator","operation":"tab","anchor":"color"})).unwrap();
    let mut h = harness(app);
    let from = tab_rect(&h, "color").center();
    let to = tab_rect(&h, "navigator").right_center() + vec2(2.0, 0.0);
    press(&mut h, from, to);
    release(&mut h, to);
    assert_eq!(group_members(layout(h.state()), "navigator"), ["navigator", "color"]);
}
#[test]
fn an_attached_full_group_can_move_sideways_without_moving_its_stacked_neighbour() {
    let mut app = compact_pair();
    app.run("window.panel.group", json!({"panel":"properties","operation":"expand"})).unwrap();
    app.run("window.panel.edge", json!({"panel":"properties","edge":"rightInner"})).unwrap();
    let mut h = harness(app);
    let actions_before =
        compat::columns(layout(h.state())).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "actions"))).unwrap().1.clone();
    let from =
        DockArea::<String>::hit_rects(&h.ctx, area_id()).into_iter().find(|(hit, _)| matches!(hit,Hit::DragBar(id) if id=="properties")).unwrap().1.center();
    let layers = body_rect(&h, "layers");
    let to = pos2(layers.left() + 1.0, layers.center().y);
    press(&mut h, from, to);
    release(&mut h, to);
    let actions = compat::columns(layout(h.state())).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "actions"))).unwrap().1;
    assert!(!actions.groups.iter().any(|g| g.panels.iter().any(|p| p == "properties")));
    assert_eq!(
        actions.groups.iter().flat_map(|g| &g.panels).collect::<Vec<_>>(),
        actions_before.groups.iter().flat_map(|g| &g.panels).filter(|p| p.as_str() != "properties").collect::<Vec<_>>()
    );
}
#[test]
fn compact_and_expanded_floating_parts_keep_independent_modes_widths_and_neighbours() {
    let mut app = full_pair();
    app.run("window.panel.group", json!({"panel":"color","operation":"collapse"})).unwrap();
    let mut h = harness(app);
    let brush_before = body_rect(&h, "brushes");
    let from = h.ctx.read_response(key("icon").with("color")).unwrap().rect.center();
    let to = pos2(brush_before.left() + 2.0, brush_before.center().y);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(5);
    let window = layout(h.state()).floating.iter().find(|w| w.contains(&"brushes".to_owned())).unwrap();
    assert!(window.contains(&"color".to_owned()));
    assert!(compat::independent_window(window));
    assert!(compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "color"));
    assert!(!compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "brushes"));
    assert!((strip_rect(&h, "color").width() - 42.0).abs() < 1.0);
    assert!((body_rect(&h, "brushes").width() - brush_before.width()).abs() < 1.1);
    let color = strip_rect(&h, "color");
    assert!(
        !h.output()
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape,egui::Shape::Rect(r) if r.rect==color && r.stroke.width>0.0 && r.stroke_kind==egui::StrokeKind::Inside)),
        "an embedded part must not add a second inside outline"
    );
    click(&mut h, area_id().with(("full-chevron", "brushes")));
    assert!(compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "color"));
    assert!(compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "brushes"));
    assert!((strip_rect(&h, "color").left() - color.left()).abs() < 1.1);
    let brush = strip_rect(&h, "brushes");
    assert!(brush.height() < 100.0, "a fully compact chain must not retain expanded height");
    click(&mut h, key("expand").with("color"));
    assert!(!compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "color"));
    assert!(compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "brushes"));
    assert!(body_rect(&h, "color").right() <= brush.left() + 1.1);
    assert!((strip_rect(&h, "brushes").left() - brush.left()).abs() < 1.1);
    let saved = crate::dock::snapshot(h.state());
    let mut restored = full_pair();
    crate::dock::apply(&mut restored, &saved);
    assert_eq!(restored.ui.panel_presentation, h.state().ui.panel_presentation);
    assert_eq!(crate::dock::snapshot(&restored), saved);
}
#[test]
fn the_left_compact_neighbour_peeks_and_expands_to_the_left_and_returns_to_its_anchor() {
    let mut app = full_pair();
    app.run("window.panel.group", json!({"panel":"color","operation":"collapse"})).unwrap();
    app.run("window.panel.edge", json!({"panel":"brushes","edge":"floating","individual":true,"collapsed":true,"x":492,"y":200,"width":320,"height":400}))
        .unwrap();
    let mut h = harness(app);
    let left = strip_rect(&h, "color");
    let right = strip_rect(&h, "brushes");
    click(&mut h, key("icon").with("color"));
    let flyout = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
    assert!(flyout.right() <= left.left() + 1.0);
    assert!(flyout.right() <= right.left());
    h.state_mut().ui.docking_flyout = None;
    h.run_steps(3);
    click(&mut h, key("expand").with("color"));
    assert!(body_rect(&h, "color").right() <= right.left() + 1.0);
    click(&mut h, area_id().with(("full-chevron", "color")));
    assert!((strip_rect(&h, "color").left() - left.left()).abs() < 1.1);
}

#[test]
fn expanded_and_flyout_controls_also_use_the_pointer_cursor() {
    let mut h = harness(full_pair());
    for part in ["full-chevron", "full-menu", "full-close"] {
        let r = h.ctx.read_response(area_id().with((part, "color"))).unwrap().rect;
        h.event(Event::PointerMoved(r.center()));
        h.run_steps(3);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::PointingHand, "{part}");
    }
    h.state_mut().run("window.panel.group", json!({"panel":"color","operation":"collapse"})).unwrap();
    h.state_mut().run("window.panel.group", json!({"panel":"color","operation":"peek"})).unwrap();
    h.run_steps(5);
    for part in ["flyout-collapse", "flyout-menu"] {
        let r = h.ctx.read_response(key(part)).unwrap().rect;
        h.event(Event::PointerMoved(r.center()));
        h.run_steps(3);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::PointingHand, "{part}");
    }
}
#[test]
fn capture_second_review_matrix() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_SECOND_REVIEW_CAPTURES").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::ProLight] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).wgpu().build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                let mut app = full_pair();
                app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
                app
            });
            h.run_steps(8);
            assert_eq!(h.state().ui.theme, theme);
            let save = |h: &mut Harness<'static, PhotocraftApp>, name: &str| {
                h.render().unwrap().save(root.join(format!("{}-{scale}-{name}.png", theme.id()))).unwrap()
            };
            save(&mut h, "full-floating-headers");
            let hover = h.ctx.read_response(area_id().with(("full-close", "color"))).unwrap().rect.center();
            h.event(Event::PointerMoved(hover));
            h.run_steps(2);
            assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::PointingHand);
            save(&mut h, "pointer-close");
            h.state_mut().run("window.panel.group", json!({"panel":"navigator","operation":"tab","anchor":"color"})).unwrap();
            h.state_mut().run("window.panel.activate", json!({"panel":"color"})).unwrap();
            h.run_steps(5);
            let a = tab_rect(&h, "color");
            let b = tab_rect(&h, "navigator");
            let from = tab_rect(&h, "brushes").center();
            press(&mut h, from, pos2(a.left() + 2.0, a.center().y));
            for (name, at) in [
                ("slot-before", pos2(a.left() + 2.0, a.center().y)),
                ("slot-between", pos2(a.right(), a.center().y)),
                ("slot-after", pos2(b.right() + 12.0, b.center().y)),
                ("separate-top", pos2(a.center().x, a.top() - 2.0)),
                ("separate-bottom", body_rect(&h, "color").center_bottom() - vec2(0.0, 2.0)),
            ] {
                h.event(Event::PointerMoved(at));
                h.run_steps(2);
                save(&mut h, name);
            }
            let canvas = body_rect(&h, super::super::CANVAS);
            h.event(Event::PointerMoved(pos2(canvas.center().x, canvas.top() + 1.0)));
            h.run_steps(2);
            save(&mut h, "top-real-width");
            h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
            h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE });
            release(&mut h, pos2(canvas.center().x, canvas.top() + 1.0));
            h.state_mut().run("window.panel.edge", json!({"panel":"navigator","edge":"rightInner","individual":true,"collapsed":true})).unwrap();
            h.state_mut().run("window.panel.group", json!({"panel":"color","operation":"collapse"})).unwrap();
            h.run_steps(5);
            let from = h.ctx.read_response(key("icon").with("color")).unwrap().rect.center();
            let to = body_rect(&h, "brushes").left_center() + vec2(2.0, 0.0);
            press(&mut h, from, to);
            save(&mut h, "compact-side-target");
            release(&mut h, to);
            h.run_steps(5);
            assert!((strip_rect(&h, "color").width() - 42.0).abs() < 1.0);
            save(&mut h, "compact-joined-full");
            click(&mut h, area_id().with(("full-chevron", "brushes")));
            save(&mut h, "both-parts-compact");
            let neighbour = strip_rect(&h, "brushes");
            click(&mut h, key("expand").with("color"));
            assert!(body_rect(&h, "color").right() <= neighbour.left() + 1.1);
            assert!((strip_rect(&h, "brushes").left() - neighbour.left()).abs() < 1.1);
            save(&mut h, "left-part-expands-outward");
            for (panel, x) in [("color", 500), ("brushes", 542)] {
                h.state_mut()
                    .run(
                        "window.panel.edge",
                        json!({"panel":panel,"edge":"floating","individual":true,"collapsed":true,"x":x,"y":200,"width":320,"height":400}),
                    )
                    .unwrap();
            }
            h.run_steps(6);
            save(&mut h, "adjacent-compact-windows");
            click(&mut h, key("icon").with("color"));
            let before = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
            assert!(before.right() <= strip_rect(&h, "color").left() + 1.0);
            save(&mut h, "left-flyout-common-header");
            let from = before.left_center() + vec2(1.0, 0.0);
            let to = from - vec2(45.0, 0.0);
            press(&mut h, from, to);
            release(&mut h, to);
            h.run_steps(3);
            assert_eq!(h.state().ui.docking_flyout.as_deref(), Some("color"));
            save(&mut h, "width-resize-stays-open");
            let r = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
            let from = r.center_bottom() - vec2(0.0, 1.0);
            let to = from + vec2(0.0, 35.0);
            press(&mut h, from, to);
            release(&mut h, to);
            h.run_steps(3);
            assert_eq!(h.state().ui.docking_flyout.as_deref(), Some("color"));
            save(&mut h, "height-resize-stays-open");
            h.state_mut().ui.docking_flyout = None;
            h.run_steps(3);
            click(&mut h, key("expand").with("color"));
            assert!(body_rect(&h, "color").right() <= strip_rect(&h, "brushes").left() + 1.0);
            save(&mut h, "left-window-expands-outward");
            click(&mut h, area_id().with(("full-chevron", "color")));
            save(&mut h, "left-window-returns-to-anchor");
        }
    }
}

#[path = "peek_transition_tests.rs"]
mod peek_transition_tests;

#[path = "seam_followup_tests.rs"]
mod seam_followup_tests;

#[path = "final_transition_tests.rs"]
mod final_transition_tests;
