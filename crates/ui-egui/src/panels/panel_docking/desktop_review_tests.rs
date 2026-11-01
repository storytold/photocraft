//! Regressions from the contributor's native Windows review of 2026-10-31.
use super::*;

#[path = "native_height_tests.rs"]
mod native_height_tests;

fn horizontal_bar(edge: &str, closed: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = full_pair();
    app.run("window.panel.edge", json!({"panel":"history","edge":edge,"individual":true,"collapsed":closed})).unwrap();
    harness(app)
}

fn group_rect(h: &Harness<'static, PhotocraftApp>, panel: &str) -> Rect {
    h.ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects"))).unwrap().into_iter().find(|(p, _)| p == panel).unwrap().1
}

fn triple(middle_compact: bool, sides_compact: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    for (panel, compact, x) in [("navigator", sides_compact, 180), ("brushes", middle_compact, 510), ("properties", sides_compact, 850)] {
        app.run("window.panel.edge", json!({"panel":panel,"edge":"floating","individual":true,"collapsed":compact,"x":x,"y":200,"width":280,"height":400}))
            .unwrap();
    }
    for (panel, anchor) in [("brushes", "navigator"), ("properties", "brushes")] {
        app.run("window.panel.group", json!({"panel":panel,"operation":"drop","whole":true,"unit":"group","to":Target::At{anchor:anchor.to_owned(),placement:Placement::Split(Zone::Right)},"sourceWidth":280,"sourceHeight":400})).unwrap();
    }
    app
}

#[test]
fn closed_horizontal_bar_drag_preview_and_detach_are_compact() {
    for edge in ["top", "bottom"] {
        let mut h = horizontal_bar(edge, true);
        let from = tab_rect(&h, "history").center();
        let to = pos2(400.0, 350.0);
        press(&mut h, from, to);
        let preview = resolve_drag(
            h.state(),
            &h.ctx,
            &DropRequest {
                panel: &"history".into(),
                pointer: to,
                targets: &[],
                floating_rect: Some(group_rect(&h, "history").translate(to - from)),
                whole_group: false,
                released: false,
                workspace: h.ctx.content_rect(),
                screen: h.ctx.content_rect(),
            },
            &mut Vec::new(),
        )
        .unwrap()
        .preview
        .unwrap();
        assert!(preview.width() <= 240.0, "a collapsed bar still drags its canvas-wide frame: {preview:?}");
        release(&mut h, to);
        h.run_steps(5);
        assert!(h.state().ui.panel_presentation.compact.contains(&"history".to_owned()), "closed bar detached as an expanded header");
        assert!(strip_rect(&h, "history").width() <= 240.0);
        assert!(strip_rect(&h, "history").height() < 110.0);
    }
}

#[test]
fn expanded_horizontal_bar_detach_stays_an_expanded_editor() {
    let mut h = horizontal_bar("top", false);
    let from = tab_rect(&h, "history").center();
    let to = pos2(400.0, 350.0);
    press(&mut h, from, to);
    release(&mut h, to);
    assert!(!h.state().ui.panel_presentation.compact.contains(&"history".to_owned()));
    let frame = group_rect(&h, "history");
    assert!(frame.width() <= 520.0 && frame.height() >= 120.0, "invalid normal editor size: {frame:?}");
}

#[test]
fn expanded_source_can_be_dropped_below_a_closed_header() {
    let mut h = horizontal_bar("top", true);
    let before = group_rect(&h, "history");
    let from = tab_rect(&h, "brushes").center();
    let to = before.center_bottom() - vec2(0.0, 1.0);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(5);
    assert!(!group_members(layout(h.state()), "history").contains(&"brushes".to_owned()), "bottom edge merged a tab instead of making a lower group");
    assert!(!floats(layout(h.state()), "brushes"), "bottom drop did not dock");
    assert!(group_rect(&h, "brushes").top() >= group_rect(&h, "history").bottom() - 1.1, "bottom drop was placed above the receiving header");
}

#[test]
fn resizing_a_middle_part_preserves_its_neighbours_widths() {
    for (middle_compact, sides_compact) in [(false, true), (true, false), (false, false)] {
        let mut h = harness(triple(middle_compact, sides_compact));
        let before: Vec<_> = ["navigator", "brushes", "properties"].map(|p| group_rect(&h, p)).into();
        let from = pos2((before[1].right() + before[2].left()) * 0.5, before[1].center().y);
        let to = from + vec2(55.0, 0.0);
        press(&mut h, from, to);
        release(&mut h, to);
        h.run_steps(5);
        let after: Vec<_> = ["navigator", "brushes", "properties"].map(|p| group_rect(&h, p)).into();
        assert!(
            (after[1].width() - before[1].width() - 55.0).abs() < 2.0,
            "middle width did not resize independently ({middle_compact},{sides_compact}): {before:?} -> {after:?}"
        );
        for i in [0, 2] {
            assert!((after[i].width() - before[i].width()).abs() < 1.1, "neighbour width changed ({middle_compact},{sides_compact}): {before:?} -> {after:?}");
        }
        assert!(after[0].right() <= after[1].left() && after[1].right() <= after[2].left());
    }
}

#[test]
fn a_closed_bar_does_not_paint_an_active_editor_tab() {
    let mut h = horizontal_bar("top", true);
    h.event(Event::PointerMoved(pos2(1000.0, 600.0)));
    h.run_steps(4);
    let tab = tab_rect(&h, "history");
    let frame = group_rect(&h, "history");
    let scale = h.ctx.pixels_per_point();
    let image = h.render().unwrap();
    let pixel = |p: Pos2| image.get_pixel((p.x * scale) as u32, (p.y * scale) as u32).0;
    assert_eq!(
        pixel(tab.right_bottom() - vec2(3.0, 3.0)),
        pixel(pos2(frame.center().x, tab.bottom() - 3.0)),
        "closed panel looks like an open selected editor"
    );
}

#[test]
fn clicking_a_closed_horizontal_tab_reopens_its_editor() {
    let mut h = horizontal_bar("top", true);
    let at = tab_rect(&h, "history").center();
    super::super::tests::click(&mut h, at);
    assert!(!super::super::super::collapsed_members(h.state(), layout(h.state())).contains("history"));
    assert!(body_rect(&h, "history").height() >= 100.0);
}

#[test]
fn stock_float_actions_preserve_a_closed_bars_compact_mode() {
    for whole in [false, true] {
        let mut h = horizontal_bar("top", true);
        let rect = [300.0, 300.0, 42.0, 68.0];
        let action = if whole {
            Action::MoveGroup { panel: "history".into(), to: Target::Float { rect: Some(rect) } }
        } else {
            Action::Float { panel: "history".into(), rect }
        };
        let params = compact_detach(h.state(), layout(h.state()), &action).unwrap();
        h.state_mut().run("window.panel.group", params).unwrap();
        h.run_steps(5);
        assert!(h.state().ui.panel_presentation.compact.contains(&"history".to_owned()));
        assert!(strip_rect(&h, "history").height() < 110.0);
    }
}

#[test]
fn expanded_floating_parts_have_one_visible_seam() {
    let mut h = harness(triple(false, false));
    let a = group_rect(&h, "navigator");
    let b = group_rect(&h, "brushes");
    super::seam_followup_tests::assert_one_border_band(&mut h, (a.right() + b.left()) * 0.5, a.center().y);
}

fn full_bars(left: bool, scale: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ProMedium);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("prefs.set", json!({"path":"interface.theme","value":crate::theme::ThemeKind::ProMedium.id()})).unwrap();
        for panel in ["history", "navigator"] {
            app.run("window.panel.edge", json!({"panel":panel,"edge":"top","individual":true,"collapsed":true})).unwrap();
        }
        if left {
            app.run("window.panel.edge", json!({"panel":"properties","edge":"left","individual":true,"collapsed":true})).unwrap();
        }
        app
    });
    h.run_steps(8);
    h
}

#[test]
fn tools_and_horizontal_rows_have_single_seams_without_crossing_the_left_column() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for left in [false, true] {
            let mut h = full_bars(left, scale);
            let bounds = h.ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap();
            let a = group_rect(&h, "history");
            let b = group_rect(&h, "navigator");
            let y = (a.bottom() + b.top()) * 0.5;
            let image = h.render().unwrap();
            let border = crate::theme::Tokens::get(&h.ctx).card_border.to_array();
            let x = (bounds.left() * scale).round() as u32;
            let py = (bounds.center().y * scale).round() as u32;
            let count = (x.saturating_sub(3)..=x + 3).filter(|x| image.get_pixel(*x, py).0 == border).count();
            assert_eq!(count, 1, "Tools separator at scale{scale}, left{left}");
            let px = (a.center().x * scale).round() as u32;
            let py = (y * scale).round() as u32;
            let count = (py.saturating_sub(2)..=py + 2).filter(|y| image.get_pixel(px, *y).0 == border).count();
            assert_eq!(count, 1, "horizontal seam at scale{scale}, left{left}");
            if left {
                let strip = strip_rect(&h, "properties");
                assert!(a.left() >= strip.right() - 1.0 && b.left() >= strip.right() - 1.0, "rows overlap the side column");
                let x = ((strip.right() - 3.0) * scale) as u32;
                assert_eq!(image.get_pixel(x, py), image.get_pixel(x, py + 2), "horizontal line crosses Properties");
            }
        }
    }
}

#[test]
fn middle_part_resize_preview_and_escape_do_not_commit_or_change_neighbour_widths() {
    let mut h = harness(triple(false, false));
    let saved = crate::dock::snapshot(h.state());
    let before: Vec<_> = ["navigator", "brushes", "properties"].map(|p| group_rect(&h, p)).into();
    let from = before[1].right_center() - vec2(0.25, 0.0);
    press(&mut h, from, from + vec2(45.0, 0.0));
    h.run_steps(2);
    assert!((group_rect(&h, "brushes").width() - before[1].width() - 45.0).abs() < 1.1);
    for i in [0, 2] {
        let panel = if i == 0 { "navigator" } else { "properties" };
        assert!((group_rect(&h, panel).width() - before[i].width()).abs() < 1.1);
    }
    h.key_press(egui::Key::Escape);
    release(&mut h, from + vec2(45.0, 0.0));
    assert_eq!(crate::dock::snapshot(h.state()), saved, "cancelled width preview was saved");
}

#[test]
fn force_floating_a_closed_bar_keeps_the_compact_presentation() {
    let mut h = horizontal_bar("top", true);
    let from = tab_rect(&h, "history").center();
    let to = pos2(400.0, 350.0);
    let modifiers = egui::Modifiers::CTRL;
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    for step in 1..=5 {
        h.event(Event::PointerMoved(from + (to - from) * (step as f32 / 5.0)));
        h.run_steps(1);
    }
    assert!(h.ctx.input(|i| i.modifiers.ctrl), "the test must exercise force-floating");
    let feedback = resolve_drag(
        h.state(),
        &h.ctx,
        &DropRequest {
            panel: &"history".into(),
            pointer: to,
            targets: &[],
            floating_rect: Some(group_rect(&h, "history").translate(to - from)),
            whole_group: false,
            released: false,
            workspace: h.ctx.content_rect(),
            screen: h.ctx.content_rect(),
        },
        &mut Vec::new(),
    )
    .unwrap();
    assert!(feedback.preview.unwrap().width() <= 240.0);
    h.event(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(5);
    assert!(h.state().ui.panel_presentation.compact.contains(&"history".to_owned()));
    assert!(strip_rect(&h, "history").height() < 110.0);
}

#[test]
fn middle_part_left_resize_survives_save_restore_and_keeps_content_clickable() {
    let mut h = harness(triple(false, false));
    let before = group_rect(&h, "brushes");
    let from = before.left_center() + vec2(0.25, 0.0);
    let to = from - vec2(45.0, 0.0);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(5);
    let after: Vec<_> = ["navigator", "brushes", "properties"].map(|p| group_rect(&h, p)).into();
    assert!((after[1].width() - before.width() - 45.0).abs() < 1.1);
    let saved = crate::dock::snapshot(h.state());
    crate::dock::apply(h.state_mut(), &saved);
    h.run_steps(5);
    for (i, panel) in ["navigator", "brushes", "properties"].into_iter().enumerate() {
        assert!((group_rect(&h, panel).width() - after[i].width()).abs() < 1.1);
    }
    click(&mut h, area_id().with(("full-chevron", "brushes")));
    assert!(h.state().ui.panel_presentation.compact.contains(&"brushes".to_owned()));
}

#[test]
fn keyboard_part_width_resizing_preserves_neighbours_and_stops_at_the_workspace_edge() {
    let mut h = harness(triple(false, false));
    let id = key("part-width").with(("brushes".to_owned(), false));
    let before: Vec<_> = ["navigator", "brushes", "properties"].map(|p| group_rect(&h, p)).into();
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(4);
    let after: Vec<_> = ["navigator", "brushes", "properties"].map(|p| group_rect(&h, p)).into();
    assert!((after[1].width() - before[1].width() - 8.0).abs() < 1.1);
    for i in [0, 2] {
        assert!((after[i].width() - before[i].width()).abs() < 1.1);
    }
    let bounds = h.ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap();
    let frame = after.into_iter().reduce(Rect::union).unwrap();
    h.state_mut()
        .run(
            "window.panel.layout",
            json!({"action":Action::MoveFloating {panel:"navigator".to_owned(),rect:[bounds.right()-frame.width(),frame.top(),frame.width(),frame.height()]}}),
        )
        .unwrap();
    h.run_steps(5);
    let before = group_rect(&h, "brushes");
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(4);
    assert!((group_rect(&h, "brushes").width() - before.width()).abs() < 1.1, "keyboard resize grew past the workspace edge");
}

#[test]
fn capture_desktop_review_matrix() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_DESKTOP_REVIEW_CAPTURES").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::ProLight] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let make = |app: PhotocraftApp| {
                let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).wgpu().build_eframe(|cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                    let mut app = app;
                    app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
                    app
                });
                h.run_steps(8);
                h
            };
            let save = |h: &mut Harness<'static, PhotocraftApp>, name: &str| {
                h.render().unwrap().save(root.join(format!("{}-{scale}-{name}.png", theme.id()))).unwrap();
            };
            let mut h = make(triple(false, false));
            let a = group_rect(&h, "navigator");
            let b = group_rect(&h, "brushes");
            super::seam_followup_tests::assert_one_border_band(&mut h, (a.right() + b.left()) * 0.5, a.center().y);
            save(&mut h, "expanded-parts");
            let from = b.right_center() - vec2(0.25, 0.0);
            press(&mut h, from, from + vec2(45.0, 0.0));
            save(&mut h, "middle-resize-preview");
            release(&mut h, from + vec2(45.0, 0.0));
            save(&mut h, "middle-resized");
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            for panel in ["history", "navigator"] {
                app.run("window.panel.edge", json!({"panel":panel,"edge":"top","individual":true,"collapsed":true})).unwrap();
            }
            app.run("window.panel.edge", json!({"panel":"properties","edge":"left","individual":true,"collapsed":true})).unwrap();
            let mut h = make(app);
            h.event(Event::PointerMoved(pos2(1100.0, 650.0)));
            h.run_steps(3);
            save(&mut h, "closed-rows-and-side");
            let from = tab_rect(&h, "history").center();
            let to = pos2(400.0, 350.0);
            press(&mut h, from, to);
            save(&mut h, "closed-row-drag");
            release(&mut h, to);
            save(&mut h, "detached-compact");
        }
    }
}
