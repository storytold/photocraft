//! Third review: repeated moves and transitions between palette, editor and floating window.
use super::*;

fn linked_app() -> PhotocraftApp {
    let mut app = full_pair();
    app.run("window.panel.group", json!({"panel":"brushes","operation":"collapse"})).unwrap();
    app.run("window.panel.group",json!({"panel":"color","operation":"drop","whole":false,"to":Target::At{anchor:"brushes".to_owned(),placement:Placement::Split(Zone::Left)},"sourceWidth":320.0})).unwrap();
    app
}
fn linked() -> Harness<'static, PhotocraftApp> {
    harness(linked_app())
}
#[test]
fn reordering_a_part_right_left_repeatedly_never_adds_its_width_again() {
    for panel in ["brushes", "color"] {
        let mut h = linked();
        let initial = layout(h.state()).floating.iter().find(|w| w.contains(&"brushes".to_owned())).unwrap().rect;
        let anchor = if panel == "brushes" { "color" } else { "brushes" };
        for zone in [Zone::Right, Zone::Left, Zone::Right, Zone::Left] {
            h.state_mut().run("window.panel.group",json!({"panel":panel,"operation":"drop","whole":true,"unit":"group","to":Target::At{anchor:anchor.to_owned(),placement:Placement::Split(zone)},"sourceWidth":if panel=="color"{320.0}else{42.0}})).unwrap();
            h.run_steps(4);
            let current = layout(h.state()).floating.iter().find(|w| w.contains(&"brushes".to_owned())).unwrap().rect;
            assert!((current[2] - initial[2]).abs() < 1.0, "{panel}/{zone:?}: {current:?} grew from {initial:?}");
        }
    }
}
#[test]
fn opening_an_editor_from_a_fully_compact_chain_uses_its_normal_size() {
    let mut h = linked();
    click(&mut h, area_id().with(("full-chevron", "color")));
    assert!(strip_rect(&h, "color").height() < 100.0);
    click(&mut h, key("icon").with("color"));
    let editor = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
    assert!((editor.width() - 320.0).abs() < 1.1 && (editor.height() - 400.0).abs() < 1.1, "editor inherited compact owner size: {editor:?}");
}
#[test]
fn dragging_the_open_editor_out_of_a_palette_creates_an_expanded_window() {
    let mut app = full_pair();
    app.run("window.panel.group", json!({"panel":"color","operation":"collapse"})).unwrap();
    let mut h = harness(app);
    click(&mut h, key("icon").with("color"));
    let editor = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
    let from = h.ctx.read_response(key("flyout-drag")).unwrap().rect.center();
    let to = pos2(250.0, 600.0);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(5);
    assert!(!compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "color"), "an open editor must not turn into an icon on tear-off");
    let window = layout(h.state()).floating.iter().find(|w| w.contains(&"color".to_owned())).unwrap();
    assert!((window.rect[2] - editor.width()).abs() < 1.1 && (window.rect[3] - editor.height()).abs() < 1.1);
    assert!(super::super::super::last_bodies(&h.ctx).iter().any(|(id, _)| id == "color"));
}
#[test]
fn an_attached_right_editor_opens_strictly_left_of_its_own_icon() {
    let mut app = full_pair();
    app.run("window.panel.edge", json!({"panel":"channels","edge":"rightOuter","individual":true,"collapsed":true})).unwrap();
    let mut h = harness(app);
    let owner = strip_rect(&h, "channels");
    click(&mut h, key("icon").with("channels"));
    let editor = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
    assert!((editor.right() - owner.left()).abs() < 1.1, "right editor {editor:?} overlaps owner {owner:?}");
}

#[test]
fn editor_resize_roundtrips_both_axes_without_growing_the_compact_owner() {
    let mut h = linked();
    click(&mut h, area_id().with(("full-chevron", "color")));
    click(&mut h, key("icon").with("color"));
    let owner = layout(h.state()).floating.iter().find(|w| w.contains(&"color".to_owned())).unwrap().rect;
    for (bottom, delta) in [(false, 280.0), (true, 100.0), (false, -180.0), (true, -90.0)] {
        let before = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
        let from = h.ctx.read_response(key("editor-edge").with(("color", bottom))).unwrap().rect.center();
        let left = from.x < before.center().x;
        let to = from + if bottom { vec2(0.0, delta) } else { vec2(if left { -delta } else { delta }, 0.0) };
        press(&mut h, from, to);
        let preview = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
        release(&mut h, to);
        h.run_steps(5);
        let after = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
        assert_eq!(h.state().ui.docking_flyout.as_deref(), Some("color"));
        assert!(
            (after.width() - preview.width()).abs() < 1.1 && (after.height() - preview.height()).abs() < 1.1,
            "resize snapped back: {preview:?} -> {after:?}"
        );
        assert!((after.width() - before.width() - if bottom { 0.0 } else { delta }).abs() < 1.1);
        assert!((after.height() - before.height() - if bottom { delta } else { 0.0 }).abs() < 1.1);
        assert_eq!(layout(h.state()).floating.iter().find(|w| w.contains(&"color".to_owned())).unwrap().rect, owner);
    }
    let saved = crate::dock::snapshot(h.state());
    let mut restored = full_pair();
    crate::dock::apply(&mut restored, &saved);
    assert_eq!(restored.ui.panel_presentation.flyouts, h.state().ui.panel_presentation.flyouts);
}
#[test]
fn detach_then_reattach_does_not_leave_width_behind_in_the_old_owner() {
    let mut h = linked();
    let original = body_rect(&h, "color").width();
    for zone in [Zone::Right, Zone::Left, Zone::Right, Zone::Left] {
        let from = h.ctx.read_response(key("grip").with("brushes")).unwrap().rect.center();
        let to = pos2(200.0, 500.0);
        press(&mut h, from, to);
        release(&mut h, to);
        h.run_steps(4);
        assert!((body_rect(&h, "color").width() - original).abs() < 1.1, "remaining full pane inherited departed part width");
        let from = h.ctx.read_response(key("icon").with("brushes")).unwrap().rect.center();
        let target = body_rect(&h, "color");
        let to = if zone == Zone::Left { target.left_center() + vec2(2.0, 0.0) } else { target.right_center() - vec2(2.0, 0.0) };
        press(&mut h, from, to);
        release(&mut h, to);
        h.run_steps(4);
        assert!((body_rect(&h, "color").width() - original).abs() < 1.1);
    }
}

#[test]
fn tearing_an_editor_from_a_compact_chain_keeps_the_sibling_compact() {
    let mut h = linked();
    click(&mut h, area_id().with(("full-chevron", "color")));
    click(&mut h, key("icon").with("color"));
    let editor = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
    let from = h.ctx.read_response(key("flyout-drag")).unwrap().rect.center();
    let to = pos2(200.0, 550.0);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(5);
    let windows = &layout(h.state()).floating;
    let full = windows.iter().find(|w| w.contains(&"color".to_owned())).unwrap();
    let mini = windows.iter().find(|w| w.contains(&"brushes".to_owned())).unwrap();
    assert!(!full.contains(&"brushes".to_owned()));
    assert!((full.rect[2] - editor.width()).abs() < 1.1 && (full.rect[3] - editor.height()).abs() < 1.1);
    assert!(!compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "color"));
    assert!(compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "brushes"));
    assert!(mini.contains(&"brushes".to_owned()));
    assert!(strip_rect(&h, "brushes").height() < 100.0);
}

#[test]
fn capture_third_review_transitions() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_THIRD_REVIEW_CAPTURES").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::ProLight] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).wgpu().build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                let mut app = linked_app();
                app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
                app
            });
            h.run_steps(8);
            let save = |h: &mut Harness<'static, PhotocraftApp>, name: &str| {
                h.render().unwrap().save(root.join(format!("{}-{scale}-{name}.png", theme.id()))).unwrap()
            };
            save(&mut h, "linked-normal");
            let initial = body_rect(&h, "color").width();
            for (name, zone) in [("reorder-left", Zone::Left), ("reorder-right", Zone::Right)] {
                h.state_mut().run("window.panel.group",json!({"panel":"brushes","operation":"drop","whole":true,"unit":"group","to":Target::At{anchor:"color".to_owned(),placement:Placement::Split(zone)},"sourceWidth":42.0})).unwrap();
                h.run_steps(5);
                assert!((body_rect(&h, "color").width() - initial).abs() < 1.1);
                save(&mut h, name);
            }
            click(&mut h, area_id().with(("full-chevron", "color")));
            save(&mut h, "fully-compact");
            click(&mut h, key("icon").with("color"));
            let editor = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
            assert!((editor.height() - 400.0).abs() < 1.1);
            save(&mut h, "normal-editor");
            for (name, delta) in [("grow", vec2(280.0, 95.0)), ("shrink", vec2(-180.0, -70.0))] {
                for bottom in [false, true] {
                    let before = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
                    let from = h.ctx.read_response(key("editor-edge").with(("color", bottom))).unwrap().rect.center();
                    let to = from + if bottom { vec2(0.0, delta.y) } else { vec2(if from.x < before.center().x { -delta.x } else { delta.x }, 0.0) };
                    press(&mut h, from, to);
                    save(&mut h, &format!("{name}-{}-preview", if bottom { "height" } else { "width" }));
                    let preview = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
                    release(&mut h, to);
                    h.run_steps(4);
                    let after = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
                    assert!((after.width() - preview.width()).abs() < 1.1 && (after.height() - preview.height()).abs() < 1.1);
                }
                save(&mut h, name);
            }
            let before = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
            let from = h.ctx.read_response(key("flyout-drag")).unwrap().rect.center();
            let to = pos2(200.0, 250.0);
            press(&mut h, from, to);
            release(&mut h, to);
            h.run_steps(5);
            assert!(!compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "color"));
            let full = layout(h.state()).floating.iter().find(|w| w.contains(&"color".to_owned())).unwrap();
            assert!((full.rect[2] - before.width()).abs() < 1.1);
            save(&mut h, "editor-tearoff");
            for edge in ["rightInner", "rightOuter"] {
                h.state_mut().run("window.panel.edge", json!({"panel":"channels","edge":edge,"individual":true,"collapsed":true})).unwrap();
                h.run_steps(6);
                let owner = strip_rect(&h, "channels");
                click(&mut h, key("icon").with("channels"));
                let editor = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
                assert!((editor.right() - owner.left()).abs() <= scale.recip() + 0.1, "{edge}: {editor:?}/{owner:?}");
                save(&mut h, edge);
                h.state_mut().ui.docking_flyout = None;
                h.run_steps(3);
            }
            h.state_mut()
                .run(
                    "window.panel.edge",
                    json!({"panel":"actions","edge":"floating","individual":true,"collapsed":true,"x":450,"y":150,"width":420,"height":400}),
                )
                .unwrap();
            h.run_steps(6);
            click(&mut h, key("icon").with("actions"));
            save(&mut h, "operations-editor");
            let from = h.ctx.read_response(key("flyout-drag")).unwrap().rect.center();
            let to = pos2(700.0, 300.0);
            press(&mut h, from, to);
            release(&mut h, to);
            h.run_steps(5);
            assert!(!compat::compact(layout(h.state()), &h.state().ui.panel_presentation, "actions"));
            save(&mut h, "operations-tearoff");
        }
    }
}
