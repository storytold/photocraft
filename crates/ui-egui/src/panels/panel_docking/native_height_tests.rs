//! Follow-up native review: document-header placement and full floating heights.
use super::*;
use crate::panel_docking::document_header;

fn compact_stack(zone: Zone) -> PhotocraftApp {
    let mut app = full_pair();
    for panel in ["color", "brushes"] {
        app.run("window.panel.group", json!({"panel":panel,"operation":"collapse"})).unwrap();
    }
    app.run("window.panel.group",json!({"panel":"brushes","operation":"drop","whole":true,"unit":"group","to":Target::At{anchor:"color".to_owned(),placement:Placement::Split(zone)},"sourceWidth":42,"sourceHeight":68})).unwrap();
    app
}

#[test]
fn expanding_a_compact_vertical_stack_restores_normal_editor_heights() {
    let mut h = harness(compact_stack(Zone::Top));
    click(&mut h, key("expand").with("brushes"));
    h.run_steps(5);
    for panel in ["brushes", "color"] {
        assert!(group_rect(&h, panel).height() >= 300.0, "{panel} expanded at miniature height: {:?}", group_rect(&h, panel));
    }
}

#[test]
fn floating_height_handles_work_before_any_width_resize() {
    for short_stored_frame in [false, true] {
        let mut app = full_pair();
        app.run("window.panel.group",json!({"panel":"brushes","operation":"drop","whole":true,"unit":"group","to":Target::At{anchor:"color".to_owned(),placement:Placement::Split(Zone::Right)},"sourceWidth":320,"sourceHeight":400})).unwrap();
        if short_stored_frame {
            app.run("window.panel.layout", json!({"action":Action::MoveFloating {panel:"color".to_owned(),rect:[180.0,180.0,85.0,68.0]}})).unwrap();
        }
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        let mut h = document_harness(app);
        let before = group_rect(&h, "brushes").union(group_rect(&h, "color"));
        let from = before.center_bottom() - vec2(0.0, 0.25);
        press(&mut h, from, from + vec2(0.0, 70.0));
        release(&mut h, from + vec2(0.0, 70.0));
        h.run_steps(4);
        let after = group_rect(&h, "brushes").union(group_rect(&h, "color"));
        assert!(
            (after.height() - before.height() - 70.0).abs() < 2.0,
            "height handle ignored before width normalization ({short_stored_frame}): {before:?} -> {after:?}"
        );
    }
}

#[test]
fn an_expanded_vertical_stack_can_resize_its_outer_bottom_before_its_width() {
    let mut app = compact_stack(Zone::Top);
    app.run("file.new", json!({"width":64,"height":64})).unwrap();
    let mut h = document_harness(app);
    click(&mut h, key("expand").with("brushes"));
    let before = group_rect(&h, "brushes").union(group_rect(&h, "color"));
    let from = pos2(before.left() + before.width() * 0.75, before.bottom() - 0.25);
    press(&mut h, from, from - vec2(0.0, 60.0));
    release(&mut h, from - vec2(0.0, 60.0));
    h.run_steps(4);
    let after = group_rect(&h, "brushes").union(group_rect(&h, "color"));
    assert!((after.height() - before.height() + 60.0).abs() < 2.0, "outer height ignored: {before:?} -> {after:?}");
}

#[test]
fn outer_bottom_corners_resize_height_without_changing_part_widths() {
    for left in [false, true] {
        let mut app = full_pair();
        app.run("window.panel.group",json!({"panel":"brushes","operation":"drop","whole":true,"unit":"group","to":Target::At{anchor:"color".to_owned(),placement:Placement::Split(Zone::Right)},"sourceWidth":320,"sourceHeight":400})).unwrap();
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        let mut h = document_harness(app);
        let parts = [group_rect(&h, "color"), group_rect(&h, "brushes")];
        let before = parts[0].union(parts[1]);
        let from = pos2(if left { before.left() + 2.0 } else { before.right() - 2.0 }, before.bottom() - 0.25);
        press(&mut h, from, from + vec2(0.0, 70.0));
        release(&mut h, from + vec2(0.0, 70.0));
        h.run_steps(4);
        let after = group_rect(&h, "color").union(group_rect(&h, "brushes"));
        assert!((after.height() - before.height() - 70.0).abs() < 2.0, "bottom corner is inert ({left}): {before:?} -> {after:?}");
        for (index, panel) in ["color", "brushes"].into_iter().enumerate() {
            assert!((group_rect(&h, panel).width() - parts[index].width()).abs() < 1.1);
        }
    }
}

fn document_harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    document_harness_with(app, crate::theme::ThemeKind::ProMedium, 1.0)
}

fn document_harness_with(app: PhotocraftApp, theme: crate::theme::ThemeKind, scale: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = app;
        app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
        app
    });
    h.run_steps(8);
    h
}

fn drop_below_documents(h: &mut Harness<'static, PhotocraftApp>, panel: &str) {
    let row = h.state().tab_strip.as_ref().unwrap().rect;
    let from = tab_rect(h, panel).center();
    let to = pos2(row.center().x, row.bottom() + 1.0);
    press(h, from, to);
    release(h, to);
    h.run_steps(5);
}

#[test]
fn document_tab_surface_is_unique_clickable_and_survives_restore_and_close() {
    let mut app = full_pair();
    for _ in 0..2 {
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
    }
    let mut h = document_harness(app);
    drop_below_documents(&mut h, "brushes");
    drop_below_documents(&mut h, "color");
    assert_eq!(layout(h.state()).panels().iter().filter(|p| p.as_str() == document_header::ID).count(), 1);
    let at = h.state().tab_strip.as_ref().unwrap().tabs.iter().find(|(doc, _)| *doc == 0).unwrap().1.center();
    super::super::tests::click(&mut h, at);
    assert_eq!(h.state().session.active_index(), Some(0));
    let saved = crate::dock::snapshot(h.state());
    crate::dock::apply(h.state_mut(), &saved);
    h.run_steps(5);
    assert_eq!(crate::dock::snapshot(h.state()), saved);
    let row = h.state().tab_strip.as_ref().unwrap().rect;
    assert!(group_rect(&h, "color").top() >= row.bottom() - 1.1);
    assert!(group_rect(&h, "color").top() < group_rect(&h, "brushes").top());
    for _ in 0..2 {
        h.state_mut().run("file.close", json!({})).unwrap();
    }
    h.run_steps(5);
    assert!(h.state().tab_strip.is_none());
    h.state_mut().run("file.new", json!({"width":64,"height":64})).unwrap();
    h.run_steps(5);
    assert!(h.state().tab_strip.is_some());
    assert!(group_rect(&h, "color").top() >= h.state().tab_strip.as_ref().unwrap().rect.bottom() - 1.1);
}

#[test]
fn panels_can_still_dock_above_the_document_row_after_below_docking() {
    let mut app = full_pair();
    app.run("file.new", json!({"width":64,"height":64})).unwrap();
    let mut h = document_harness(app);
    drop_below_documents(&mut h, "brushes");
    let row = h.state().tab_strip.as_ref().unwrap().rect;
    let from = tab_rect(&h, "color").center();
    let to = pos2(row.center().x, row.top() + 1.0);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(5);
    let row = h.state().tab_strip.as_ref().unwrap().rect;
    assert!(!floats(layout(h.state()), "color"));
    assert!(group_rect(&h, "color").bottom() <= row.top() + 1.1);
    assert!(group_rect(&h, "brushes").top() >= row.bottom() - 1.1);
}

#[test]
fn document_tab_surface_is_protected_and_rejects_malformed_placement() {
    let mut app = full_pair();
    app.run(
        "window.panel.group",
        json!({"panel":"brushes","operation":"drop","to":Target::At{anchor:document_header::ID.to_owned(),placement:Placement::Split(Zone::Bottom)}}),
    )
    .unwrap();
    let before = crate::dock::snapshot(&app);
    for action in
        [Action::Close { panel: document_header::ID.to_owned() }, Action::Float { panel: document_header::ID.to_owned(), rect: [100.0, 100.0, 320.0, 400.0] }]
    {
        assert!(app.run("window.panel.layout", json!({"action":action})).is_err());
        assert_eq!(crate::dock::snapshot(&app), before);
    }
    let mut layout = layout(&app).clone();
    layout.root = Some(crate::panel_docking::Node::Tabs { panels: vec![crate::panel_docking::CANVAS.to_owned()], active: 0 });
    layout.floating.push(craft_ui::docking::Floating::single(vec![document_header::ID.to_owned()], 0, [100.0, 100.0, 320.0, 400.0]));
    assert!(!crate::panel_docking::valid(&layout));
}

#[test]
fn control_collapse_remembers_the_original_vertical_frame_size() {
    let mut h = harness(compact_pair());
    click(&mut h, key("expand").with("properties"));
    h.run_steps(5);
    let frame = group_rect(&h, "properties").union(group_rect(&h, "actions"));
    assert!((frame.height() - 400.0).abs() < 1.1, "control collapse lost the prior 400px frame: {frame:?}");
}

#[test]
fn repeated_collapse_restore_keeps_resized_group_heights_at_fractional_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for zone in [Zone::Top, Zone::Right] {
            let mut h = document_harness_with(compact_stack(zone), crate::theme::ThemeKind::ProMedium, scale);
            for panel in ["brushes", "color"] {
                if h.state().ui.panel_presentation.compact.contains(&panel.to_owned()) {
                    click(&mut h, key("expand").with(panel));
                }
            }
            let frame = group_rect(&h, "color").union(group_rect(&h, "brushes"));
            let from = frame.center_bottom() - vec2(0.0, 0.25);
            press(&mut h, from, from - vec2(0.0, 50.0));
            release(&mut h, from - vec2(0.0, 50.0));
            h.run_steps(4);
            let before = [group_rect(&h, "brushes"), group_rect(&h, "color")];
            for _ in 0..3 {
                for panel in ["brushes", "color"] {
                    if !h.state().ui.panel_presentation.compact.contains(&panel.to_owned()) {
                        click(&mut h, area_id().with(("full-chevron", panel)));
                    }
                }
                for panel in ["brushes", "color"] {
                    if h.state().ui.panel_presentation.compact.contains(&panel.to_owned()) {
                        click(&mut h, key("expand").with(panel));
                    }
                }
                h.run_steps(4);
                for (index, panel) in ["brushes", "color"].into_iter().enumerate() {
                    let after = group_rect(&h, panel);
                    assert!((after.height() - before[index].height()).abs() < 1.1, "height drift {zone:?},scale{scale}: {:?} -> {after:?}", before[index]);
                }
            }
        }
    }
}

#[test]
fn malformed_normal_group_sizes_fail_atomically() {
    for sizes in [json!(5), json!({"color":[320,-1]}), json!({"unknown":[320,400]})] {
        let mut app = full_pair();
        let saved = crate::dock::snapshot(&app);
        assert!(app.run("window.panel.group", json!({"panel":"color","operation":"collapse","normalSizes":sizes})).is_err());
        assert_eq!(crate::dock::snapshot(&app), saved);
    }
}

#[test]
fn stacked_group_sizes_restore_independently_of_editor_sizes_and_invalid_preferences() {
    let saved = json!({"panelPresentation":{
        "compact":["color"],
        "floating_sizes":{"color":[320,400]},
        "stacked_sizes":{"color":[320,200],"brushes":[320,-1],"unknown":[320,400]},
        "widths":"damaged"
    }});
    let presentation = crate::panel_docking::compat::presentation(&saved);
    assert_eq!(presentation.stacked_sizes.len(), 1);
    assert_eq!(presentation.stacked_sizes.get("color"), Some(&[320.0, 200.0]));
    assert_eq!(presentation.floating_sizes.get("color"), Some(&[320.0, 400.0]));
    assert_eq!(presentation.compact, ["color"]);
    let roundtrip = crate::panel_docking::compat::presentation(&json!({"panelPresentation":presentation}));
    assert_eq!(roundtrip, presentation);
}

#[test]
fn protected_document_row_keeps_reorder_and_large_tab_preferences() {
    let mut app = full_pair();
    for _ in 0..2 {
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
    }
    let mut h = document_harness(app);
    drop_below_documents(&mut h, "brushes");
    let name = h.state().session.documents().first().unwrap().doc.name.clone();
    let tabs = &h.state().tab_strip.as_ref().unwrap().tabs;
    let from = tabs.iter().find(|(doc, _)| *doc == 0).unwrap().1.center();
    let to = tabs.iter().find(|(doc, _)| *doc == 1).unwrap().1.right_center() - vec2(2.0, 0.0);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(5);
    assert_eq!(h.state().session.documents().last().unwrap().doc.name, name);
    h.state_mut().session.prefs.edit(|p| p.workspace.large_tabs = true);
    h.run_steps(4);
    let row = h.state().tab_strip.as_ref().unwrap().rect;
    assert!((row.height() - 34.0).abs() < 0.1);
    assert!(group_rect(&h, "brushes").top() >= row.bottom() - 1.1);
}

#[test]
fn capture_native_height_and_document_header_review() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_NATIVE_HEIGHT_CAPTURES").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::ProLight] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let save = |h: &mut Harness<'static, PhotocraftApp>, name: &str| {
                h.render().unwrap().save(root.join(format!("{}-{scale}-{name}.png", theme.id()))).unwrap();
            };
            let mut app = full_pair();
            app.run("file.new", json!({"width":400,"height":300})).unwrap();
            app.run("window.panel.close", json!({"panel":"color"})).unwrap();
            let mut h = document_harness_with(app, theme, scale);
            let row = h.state().tab_strip.as_ref().unwrap().rect;
            let from = tab_rect(&h, "brushes").center();
            let to = pos2(row.center().x, row.bottom() + 1.0);
            press(&mut h, from, to);
            save(&mut h, "under-documents-preview");
            release(&mut h, to);
            h.event(Event::PointerMoved(pos2(1200.0, 760.0)));
            h.run_steps(5);
            save(&mut h, "under-documents-docked");
            let row = h.state().tab_strip.as_ref().unwrap().rect;
            assert!(group_rect(&h, "brushes").top() >= row.bottom() - 1.1);
            let border = crate::theme::Tokens::get(&h.ctx).card_border.to_array();
            let image = h.render().unwrap();
            let x = ((row.right() - 100.0) * scale).round() as u32;
            let y = (((row.bottom() + group_rect(&h, "brushes").top()) * 0.5) * scale).round() as u32;
            let count = (y.saturating_sub(2)..=y + 2).filter(|y| image.get_pixel(x, *y).0 == border).count();
            assert_eq!(count, 1, "document header seam, theme{} scale{scale}", theme.id());
            click(&mut h, area_id().with(("full-chevron", "brushes")));
            save(&mut h, "under-documents-collapsed");
            let mut h = document_harness_with(compact_stack(Zone::Top), theme, scale);
            save(&mut h, "vertical-compact");
            click(&mut h, key("expand").with("brushes"));
            h.run_steps(5);
            save(&mut h, "vertical-expanded");
            for p in ["brushes", "color"] {
                assert!(group_rect(&h, p).height() >= 300.0);
            }
            click(&mut h, area_id().with(("full-chevron", "brushes")));
            save(&mut h, "vertical-collapsed-again");
            click(&mut h, key("expand").with("brushes"));
            h.run_steps(5);
            save(&mut h, "vertical-restored");
            let mut h = document_harness_with(compact_stack(Zone::Right), theme, scale);
            for panel in ["brushes", "color"] {
                click(&mut h, key("expand").with(panel));
            }
            save(&mut h, "horizontal-expanded");
            let before = group_rect(&h, "color").union(group_rect(&h, "brushes"));
            let from = before.right_bottom() - vec2(2.0, 0.25);
            let to = from + vec2(0.0, 30.0);
            press(&mut h, from, to);
            save(&mut h, "bottom-corner-preview");
            release(&mut h, to);
            h.run_steps(4);
            save(&mut h, "bottom-corner-resized");
            let after = group_rect(&h, "color").union(group_rect(&h, "brushes"));
            assert!((after.height() - before.height() - 30.0).abs() < 2.0);
            assert!((after.width() - before.width()).abs() < 1.1);
        }
    }
}

#[test]
fn a_panel_docks_immediately_below_the_document_tab_row() {
    let mut app = full_pair();
    app.run("file.new", json!({"width":64,"height":64})).unwrap();
    let mut h = document_harness(app);
    let tabs = h.state().tab_strip.as_ref().unwrap().rect;
    let from = tab_rect(&h, "brushes").center();
    let to = pos2(tabs.center().x, tabs.bottom() + 1.0);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(5);
    assert!(!floats(layout(h.state()), "brushes"));
    let after_tabs = h.state().tab_strip.as_ref().unwrap().rect;
    let panel = group_rect(&h, "brushes");
    assert!((after_tabs.top() - tabs.top()).abs() < 1.1, "document tabs moved below the docked panel");
    assert!(panel.top() >= after_tabs.bottom() - 1.1);
    assert!(h.state().last_canvas_rect.top() >= panel.bottom() - 1.1);
}
