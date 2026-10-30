//! Final review: incoming full size and persistent horizontal tab bars.
use super::*;
#[test]
fn a_full_float_keeps_its_height_when_joining_and_leaving_a_short_compact_receiver() {
    for height in [400.0, 520.0] {
        let mut app = full_pair();
        app.run("window.panel.edge", json!({"panel":"brushes","edge":"floating","individual":true,"collapsed":true,"x":850,"y":180,"width":320,"height":80}))
            .unwrap();
        app.run("window.panel.layout", json!({"action":Action::MoveFloating{panel:"color".to_owned(),rect:[450.0,200.0,320.0,height]}})).unwrap();
        let mut h = harness(app);
        let before = body_rect(&h, "color").height();
        let from = tab_rect(&h, "color").center();
        let to = strip_rect(&h, "brushes").left_center() + vec2(1.0, 0.0);
        press(&mut h, from, to);
        release(&mut h, to);
        h.run_steps(5);
        assert!((body_rect(&h, "color").height() - before).abs() < 1.1, "full source inherited mini height");
        let from = tab_rect(&h, "color").center();
        let to = pos2(200.0, 300.0);
        press(&mut h, from, to);
        release(&mut h, to);
        h.run_steps(5);
        let full = layout(h.state()).floating.iter().find(|w| w.contains(&"color".to_owned())).unwrap();
        assert!(!full.contains(&"brushes".to_owned()));
        assert!((body_rect(&h, "color").height() - before).abs() < 1.1, "detach lost original full height");
    }
}
fn top_bar() -> Harness<'static, PhotocraftApp> {
    let mut app = full_pair();
    app.run("window.panel.edge", json!({"panel":"history","edge":"floating","individual":true,"collapsed":true,"x":700,"y":150,"width":320,"height":400}))
        .unwrap();
    let mut h = harness(app);
    let canvas = body_rect(&h, super::super::super::CANVAS);
    let from = h.ctx.read_response(key("grip").with("history")).unwrap().rect.center();
    let to = pos2(canvas.center().x, canvas.top() + 1.0);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(6);
    h
}
#[test]
fn a_top_tab_bar_expands_and_collapses_in_place_instead_of_becoming_a_float() {
    let mut h = top_bar();
    assert!(!floats(layout(h.state()), "history"));
    h.state_mut().run("window.panel.group", json!({"panel":"history","operation":"expand"})).unwrap();
    h.run_steps(5);
    click(&mut h, area_id().with(("full-chevron", "history")));
    assert!(!floats(layout(h.state()), "history"), "collapse floated a top-attached tab group");
    assert!(super::super::super::collapsed_members(h.state(), layout(h.state())).contains("history"));
}
#[test]
fn a_collapsed_top_bar_accepts_a_tab_at_the_visible_insertion_slot_and_stays_closed() {
    let mut h = top_bar();
    let tab = tab_rect(&h, "history");
    let from = tab_rect(&h, "color").center();
    let to = tab.right_center() + vec2(2.0, 0.0);
    press(&mut h, from, to);
    release(&mut h, to);
    h.run_steps(6);
    assert_eq!(group_members(layout(h.state()), "history"), ["history", "color"]);
    assert!(!floats(layout(h.state()), "color"));
    let closed = super::super::super::collapsed_members(h.state(), layout(h.state()));
    assert!(closed.contains("history") && closed.contains("color"), "tab insertion expanded a closed bar");
}

#[test]
fn closed_horizontal_bars_offer_before_between_and_after_tab_slots() {
    for slot in 0..3 {
        let mut h = top_bar();
        h.state_mut().run("window.panel.group", json!({"panel":"color","operation":"tab","anchor":"history"})).unwrap();
        h.run_steps(5);
        let a = tab_rect(&h, "history");
        let b = tab_rect(&h, "color");
        let x = match slot {
            0 => a.left() + 2.0,
            1 => a.right(),
            _ => b.right() + 2.0,
        };
        let to = pos2(x, a.center().y);
        let from = tab_rect(&h, "brushes").center();
        press(&mut h, from, to);
        release(&mut h, to);
        h.run_steps(5);
        let mut expected = vec!["history".to_owned(), "color".to_owned()];
        expected.insert(slot, "brushes".into());
        assert_eq!(group_members(layout(h.state()), "history"), expected);
        let closed = super::super::super::collapsed_members(h.state(), layout(h.state()));
        assert!(expected.iter().all(|p| closed.contains(p)));
    }
}
#[test]
fn explicit_horizontal_edge_compaction_roundtrips_as_a_tab_bar() {
    for edge in ["top", "bottom"] {
        let mut app = full_pair();
        app.run("window.panel.edge", json!({"panel":"color","edge":edge,"individual":true,"collapsed":true})).unwrap();
        let mut h = harness(app);
        let frame = |h: &Harness<'static, PhotocraftApp>| {
            h.ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects"))).unwrap().into_iter().find(|(p, _)| p == "color").unwrap().1
        };
        assert!((frame(&h).height() - super::super::super::contract::FULL_HEADER_HEIGHT).abs() < 1.1);
        click(&mut h, area_id().with(("full-chevron", "color")));
        assert!(frame(&h).height() > 100.0);
        click(&mut h, area_id().with(("full-chevron", "color")));
        assert!((frame(&h).height() - super::super::super::contract::FULL_HEADER_HEIGHT).abs() < 1.1);
        let saved = crate::dock::snapshot(h.state());
        crate::dock::apply(h.state_mut(), &saved);
        h.run_steps(5);
        assert!(!floats(layout(h.state()), "color"));
        assert!((frame(&h).height() - super::super::super::contract::FULL_HEADER_HEIGHT).abs() < 1.1);
    }
}

#[test]
fn capture_final_review_transitions() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_FINAL_REVIEW_CAPTURES").map(std::path::PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::ProLight] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_pixels_per_point(scale).wgpu().build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                let mut app = full_pair();
                app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
                app.run(
                    "window.panel.edge",
                    json!({"panel":"brushes","edge":"floating","individual":true,"collapsed":true,"x":650,"y":180,"width":320,"height":80}),
                )
                .unwrap();
                app
            });
            h.run_steps(8);
            let save = |h: &mut Harness<'static, PhotocraftApp>, name: &str| {
                h.run_steps(24);
                let stem = format!("{}-{scale}-{name}", theme.id());
                h.render().unwrap().save(root.join(format!("{stem}.png"))).unwrap();
                let rects: std::collections::BTreeMap<_, _> = h
                    .ctx
                    .data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects")))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(p, r)| (p, [r.left(), r.top(), r.width(), r.height()]))
                    .collect();
                std::fs::write(
                    root.join(format!("{stem}.json")),
                    serde_json::to_string_pretty(&json!({"theme":theme.id(),"scale":scale,"panels":rects})).unwrap(),
                )
                .unwrap();
            };
            save(&mut h, "full-before-join");
            let height = body_rect(&h, "color").height();
            let from = tab_rect(&h, "color").center();
            let to = strip_rect(&h, "brushes").right_center() - vec2(1.0, 0.0);
            press(&mut h, from, to);
            release(&mut h, to);
            h.run_steps(6);
            assert!((body_rect(&h, "color").height() - height).abs() < 1.1);
            save(&mut h, "full-joined-to-mini");
            let from = tab_rect(&h, "color").center();
            let to = pos2(200.0, 250.0);
            press(&mut h, from, to);
            release(&mut h, to);
            h.run_steps(6);
            assert!((body_rect(&h, "color").height() - height).abs() < 1.1);
            save(&mut h, "full-detached");
            h.state_mut()
                .run(
                    "window.panel.edge",
                    json!({"panel":"history","edge":"floating","individual":true,"collapsed":true,"x":700,"y":150,"width":320,"height":400}),
                )
                .unwrap();
            h.run_steps(5);
            let canvas = body_rect(&h, super::super::super::CANVAS);
            let from = h.ctx.read_response(key("grip").with("history")).unwrap().rect.center();
            let to = pos2(canvas.center().x, canvas.top() + 1.0);
            press(&mut h, from, to);
            release(&mut h, to);
            h.run_steps(6);
            save(&mut h, "top-collapsed");
            click(&mut h, area_id().with(("full-chevron", "history")));
            save(&mut h, "top-expanded");
            click(&mut h, area_id().with(("full-chevron", "history")));
            assert!(!floats(layout(h.state()), "history"));
            save(&mut h, "top-recollapsed");
            let to = tab_rect(&h, "history").right_center() + vec2(2.0, 0.0);
            let from = tab_rect(&h, "color").center();
            press(&mut h, from, to);
            release(&mut h, to);
            h.run_steps(6);
            assert_eq!(group_members(layout(h.state()), "history"), ["history", "color"]);
            save(&mut h, "top-added-tab");
            let to = tab_rect(&h, "history").right_center();
            let from = h.ctx.read_response(key("icon").with("brushes")).unwrap().rect.center();
            press(&mut h, from, to);
            save(&mut h, "top-between-preview");
            release(&mut h, to);
            h.run_steps(6);
            assert_eq!(group_members(layout(h.state()), "history"), ["history", "brushes", "color"]);
            save(&mut h, "top-between-inserted");
            click(&mut h, area_id().with(("full-chevron", "brushes")));
            save(&mut h, "top-tabs-expanded");
            click(&mut h, area_id().with(("full-chevron", "brushes")));
            assert!(!floats(layout(h.state()), "brushes"));
            save(&mut h, "top-tabs-collapsed");
        }
    }
}
