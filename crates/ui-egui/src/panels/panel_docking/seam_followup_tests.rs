//! Fourth review: compact joining, shared physical seams and collapse direction.
use super::*;
fn pair(compact: bool, zone: Zone) -> PhotocraftApp {
    let mut app = full_pair();
    app.run("window.panel.group", json!({"panel":"brushes","operation":"collapse"})).unwrap();
    if compact {
        app.run("window.panel.group", json!({"panel":"color","operation":"collapse"})).unwrap();
    }
    app.run("window.panel.group",json!({"panel":"brushes","operation":"drop","whole":true,"unit":"group","to":Target::At{anchor:"color".to_owned(),placement:Placement::Split(zone)},"sourceWidth":42.0})).unwrap();
    app
}
#[test]
fn joining_two_compact_floats_does_not_inherit_normal_window_height() {
    for zone in [Zone::Left, Zone::Right] {
        let mut h = harness(pair(true, zone));
        assert!(strip_rect(&h, "brushes").height() < 100.0, "mini-to-mini retained normal owner height");
        assert!(strip_rect(&h, "color").height() < 100.0);
        let saved = crate::dock::snapshot(h.state());
        crate::dock::apply(h.state_mut(), &saved);
        h.run_steps(5);
        assert!(strip_rect(&h, "brushes").height() < 100.0, "restore stretched compact chain");
    }
}
fn arrow_points_left(h: &Harness<'static, PhotocraftApp>, panel: &str) -> bool {
    let r = h.ctx.read_response(area_id().with(("full-chevron", panel))).unwrap().rect;
    h.output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Path(p) if p.points.len() == 3 && p.points.iter().all(|p| r.contains(*p)) => Some(p.points[1].x < p.points[0].x),
            _ => None,
        })
        .unwrap()
}
#[test]
fn mixed_chain_collapse_arrow_points_toward_the_compact_neighbour() {
    for zone in [Zone::Left, Zone::Right] {
        let h = harness(pair(false, zone));
        assert_eq!(arrow_points_left(&h, "color"), zone == Zone::Left, "{zone:?}: header controls' screen position is not the collapse direction");
    }
}
#[test]
fn a_flyout_and_its_owner_share_one_physical_border_band() {
    for edge in ["left", "rightInner", "rightOuter", "floating"] {
        let mut app = full_pair();
        app.run("window.panel.edge", json!({"panel":"navigator","edge":edge,"individual":true,"collapsed":true,"x":500,"y":150,"width":320,"height":400}))
            .unwrap();
        let mut h = harness(app);
        let owner = strip_rect(&h, "navigator");
        click(&mut h, key("icon").with("navigator"));
        let editor = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
        let overlap = if editor.center().x < owner.center().x { editor.right() - owner.left() } else { owner.right() - editor.left() };
        assert!((overlap - h.ctx.pixels_per_point().recip()).abs() < 0.01, "{edge}: borders are disjoint, overlap={overlap}");
    }
}

#[test]
fn a_previously_stretched_compact_chain_is_fitted_without_losing_normal_sizes() {
    let mut app = pair(true, Zone::Left);
    let sizes = app.ui.panel_presentation.floating_sizes.clone();
    app.run("window.panel.layout", json!({"action":Action::MoveFloating{panel:"color".to_owned(),rect:[450.0,200.0,85.0,600.0]}})).unwrap();
    let saved = crate::dock::snapshot(&app);
    let mut restored = full_pair();
    crate::dock::apply(&mut restored, &saved);
    let h = harness(restored);
    assert!(strip_rect(&h, "brushes").height() < 100.0);
    assert_eq!(h.state().ui.panel_presentation.floating_sizes, sizes);
}
fn assert_one_border_band(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32) {
    let scale = h.ctx.pixels_per_point();
    let border = crate::theme::Tokens::get(&h.ctx).card_border.to_array();
    let image = h.render().unwrap();
    let x = (x * scale).round() as i32;
    let y = (y * scale).round() as u32;
    let band: Vec<_> = (x - 3..=x + 3).map(|x| image.get_pixel(x as u32, y).0).collect();
    assert_eq!(band.iter().filter(|p| **p == border).count(), 1, "physical seam at {x},{y} scale{scale}: {band:?},border={border:?}");
}
#[test]
fn capture_fourth_review_seams() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_FOURTH_REVIEW_CAPTURES").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::ProLight] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for zone in [Zone::Left, Zone::Right] {
                let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).wgpu().build_eframe(|cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                    let mut app = pair(true, zone);
                    app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
                    app
                });
                h.run_steps(8);
                let save = |h: &mut Harness<'static, PhotocraftApp>, name: &str| {
                    h.render().unwrap().save(root.join(format!("{}-{scale}-{zone:?}-{name}.png", theme.id()))).unwrap()
                };
                assert!(strip_rect(&h, "brushes").height() < 100.0);
                save(&mut h, "joined-miniatures");
                click(&mut h, key("expand").with("color"));
                assert_eq!(arrow_points_left(&h, "color"), zone == Zone::Left);
                let mini = strip_rect(&h, "brushes");
                let frame = h.ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects"))).unwrap().into_iter().find(|(p, _)| p == "color").unwrap().1;
                let x = if zone == Zone::Left { mini.right() } else { frame.right() };
                save(&mut h, "mixed-expanded");
                assert_one_border_band(&mut h, x, mini.top() + 100.0);
                click(&mut h, area_id().with(("full-chevron", "color")));
                save(&mut h, "collapsed-again");
                assert!(strip_rect(&h, "brushes").height() < 100.0);
                if zone == Zone::Right {
                    continue;
                }
                for edge in ["left", "rightInner", "rightOuter", "floating"] {
                    h.state_mut()
                        .run(
                            "window.panel.edge",
                            json!({"panel":"navigator","edge":edge,"individual":true,"collapsed":true,"x":650,"y":150,"width":320,"height":400}),
                        )
                        .unwrap();
                    h.run_steps(5);
                    let mini = strip_rect(&h, "navigator");
                    click(&mut h, key("icon").with("navigator"));
                    let editor = h.ctx.data(|d| d.get_temp::<Rect>(key("flyout-rect"))).unwrap();
                    let left = editor.center().x < mini.center().x;
                    let x = if left { mini.left() } else { mini.right() - scale.recip() };
                    save(&mut h, edge);
                    assert_one_border_band(&mut h, x, (editor.top() + 50.0).min(mini.bottom() - 10.0).min(editor.bottom() - 10.0));
                    h.state_mut().ui.docking_flyout = None;
                    h.run_steps(3);
                }
            }
        }
    }
}
