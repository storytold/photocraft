use super::*;
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
}

#[test]
fn panel_commands_group_split_reorder_float_and_restore_without_duplicate_ownership() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
    app.run("window.panel.move", json!({"panel":"properties", "anchor":"swatches", "before":"swatches"})).unwrap();
    let before_float = app.ui.docking.clone().unwrap();
    app.run("window.panel.float", json!({"panel":"properties", "x":50,"y":70,"width":330,"height":410})).unwrap();
    assert!(app.ui.docking.as_ref().unwrap().floating.iter().any(|g| g.panels == ["properties"]));
    app.run("window.panel.dock", json!({"panel":"properties"})).unwrap();
    assert_eq!(app.ui.docking.as_ref().unwrap(), &before_float);
    app.run("window.panel.close", json!({"panel":"properties"})).unwrap();
    assert!(!app.ui.docking.as_ref().unwrap().contains(&"properties".into()));
    app.run("window.panel.activate", json!({"panel":"properties"})).unwrap();
    assert_eq!(app.ui.docking.as_ref().unwrap(), &before_float);
    let layout = app.ui.docking.as_ref().unwrap();
    assert!(valid(layout));
    let unique: std::collections::HashSet<_> = layout.panels().into_iter().collect();
    assert_eq!(unique.len(), layout.panels().len());
    let saved = serde_json::to_value(&app.ui).unwrap();
    let restored: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.docking, app.ui.docking);
    assert_eq!(restored.docking_hidden, app.ui.docking_hidden);
}

#[test]
fn floating_panel_rail_toggle_closes_the_float_window() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"properties","x":50,"y":70,"width":330,"height":410})).unwrap();
    assert!(app.ui.docking.as_ref().unwrap().floating.iter().any(|g| g.panels.contains(&"properties".into())));

    rail_click(&mut app, Group::Properties);

    assert!(app.ui.docking.as_ref().unwrap().floating.iter().all(|g| !g.panels.contains(&"properties".into())));
    assert!(app.ui.docking.as_ref().unwrap().contains(&"adjustments".into()));
    assert!(app.ui.panels.properties, "the docked Adjustments tab keeps the group visible");
}

#[test]
fn brushes_menu_action_uses_the_shared_docking_layout() {
    let mut app = app();
    let before = app.ui.docking.clone();
    assert!(crate::menus::invoke(&mut app, &egui::Context::default(), "window.panel.brushes", json!({})).is_ok());
    let layout = app.ui.docking.as_ref().expect("brushes panel must be registered in docking");
    assert!(layout.contains(&"brushes".to_string()));
    assert!(app.ui.panels.brushes);
    assert_ne!(app.ui.docking, before);

    app.run("window.panel.float", json!({"panel":"brushes"})).unwrap();
    assert!(app.ui.docking.as_ref().unwrap().floating.iter().any(|group| group.panels.contains(&"brushes".into())));
    app.run("window.panel.dock", json!({"panel":"brushes"})).unwrap();
    assert!(!app.ui.docking.as_ref().unwrap().floating.iter().any(|group| group.panels.contains(&"brushes".into())));
}

#[test]
fn invalid_panel_requests_are_atomic_and_do_not_enter_a_custom_workspace() {
    let mut app = app();
    for (id, params) in [
        ("window.panel.move", json!({"panel":"layers","anchor":"unknown"})),
        ("window.panel.move", json!({"panel":"layers","anchor":"properties","before":"unknown"})),
        ("window.panel.move", json!({"panel":"layers","anchor":"properties","zone":42})),
        ("window.panel.float", json!({"panel":"layers","width":-10})),
        ("window.panel.float", json!({"panel":"unknown"})),
        ("window.panel.float", json!({"panel":"layers","x":"invalid"})),
    ] {
        let old = app.ui.docking.clone();
        let hidden = app.ui.docking_hidden.clone();
        assert!(app.run(id, params).is_err(), "{id}");
        assert_eq!(app.ui.docking, old);
        assert_eq!(app.ui.docking_hidden, hidden);
    }
}

fn harness(app: PhotocraftApp, size: egui::Vec2) -> Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let mut harness = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    harness.run_steps(4);
    harness
}

fn drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn ordinary_panel_tab_drag_enters_shared_docking_and_can_redock_into_another_tab_group() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    drag(&mut h, source, egui::pos2(160.0, 140.0));
    let layout = h.state().ui.docking.as_ref().expect("ordinary tab drag starts shared docking");
    assert!(layout.floating.iter().any(|g| g.panels.contains(&"layers".into())), "{layout:?}");
    let source = h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &"layers".to_string()))).unwrap().rect.center();
    let destination = h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &"properties".to_string()))).unwrap().rect.center();
    drag(&mut h, source, destination);
    let layout = h.state().ui.docking.as_ref().unwrap();
    assert!(!layout.floating.iter().any(|g| g.panels.contains(&"layers".into())));
    assert!(valid(layout));
}

#[test]
fn brushes_tab_drag_floats_and_can_redock_into_another_tab_group() {
    let mut app = app();
    crate::menus::invoke(&mut app, &egui::Context::default(), "window.panel.brushes", json!({})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Brushes").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    drag(&mut h, source, egui::pos2(160.0, 140.0));
    let layout = h.state().ui.docking.as_ref().expect("brushes drag starts shared docking");
    assert!(layout.floating.iter().any(|group| group.panels.contains(&"brushes".into())), "{layout:?}");

    let source = h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &"brushes".to_string()))).unwrap().rect.center();
    let destination = h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &"layers".to_string()))).unwrap().rect.center();
    drag(&mut h, source, destination);
    let layout = h.state().ui.docking.as_ref().unwrap();
    assert!(!layout.floating.iter().any(|group| group.panels.contains(&"brushes".into())));
    assert!(valid(layout));
}

#[test]
fn saved_workspace_restores_the_custom_tree_and_hidden_panel_placement() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    app.run("window.panel.float", json!({"panel":"properties","x":50,"y":80})).unwrap();
    app.run("window.panel.close", json!({"panel":"swatches"})).unwrap();
    let layout = app.ui.docking.clone();
    let hidden = app.ui.docking_hidden.clone();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name":"Custom docking"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
    assert!(app.ui.docking.is_none());
    crate::menus::invoke(&mut app, &ctx, "window.workspace.select", json!({"name":"Custom docking"})).unwrap();
    assert_eq!(app.ui.docking, layout);
    assert_eq!(app.ui.docking_hidden, hidden);
}

#[test]
fn shared_layout_keeps_panel_identity_across_themes_and_respects_workspace_lock() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"color","anchor":"layers"})).unwrap();
    let layout = app.ui.docking.clone();
    app.ui.theme = crate::theme::ThemeKind::Studio;
    assert_eq!(app.ui.docking, layout);
    app.session.prefs.edit(|prefs| prefs.workspace_locked = true);
    assert!(app.run("window.panel.float", json!({"panel":"color"})).is_err());
    assert_eq!(app.ui.docking, layout);
    assert!(app.run("window.panel.activate", json!({"panel":"color"})).is_ok());
}

/// Explicit offscreen evidence; this creates no native windows.
#[test]
fn capture_custom_panel_docking_visual_fixtures() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(directory) = std::env::var_os("CRAFT_UI_DOCKING_FIXTURES").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&directory).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        for width in [800.0, 1280.0] {
            for scale in [1.0, 1.5, 2.0] {
                let mut app = app();
                app.run("file.new", json!({})).unwrap();
                app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
                app.run("window.panel.float", json!({"panel":"properties","x":30,"y":90,"width":300,"height":440})).unwrap();
                let mut ready = false;
                let mut harness = Harness::builder().with_size(vec2(width, 800.0)).with_pixels_per_point(scale).wgpu().build_ui_state(
                    move |ui, app: &mut PhotocraftApp| {
                        if !ready {
                            PhotocraftApp::setup_context(ui.ctx(), theme);
                            app.ui.theme = theme;
                            ready = true;
                            return;
                        }
                        crate::panels::right_dock(app, ui);
                        egui::CentralPanel::default().show(ui, |ui| {
                            ui.label("Custom panel workspace");
                        });
                    },
                    app,
                );
                harness.input_mut().max_texture_side = Some(8192);
                harness.run_steps(5);
                assert!(valid(harness.state().ui.docking.as_ref().unwrap()));
                harness.render().unwrap().save(directory.join(format!("photocraft-shared-docking-{}-{width}-{scale}x.png", theme.id()))).unwrap();
            }
        }
    }
}

#[test]
fn floating_group_titles_are_opaque_in_studio_themes() {
    let _gpu = photocraft_testkit::gpu_lock();
    // Studio's tab strip is transparent over the dock's card; a floating group must not show the canvas (#4509).
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"swatches","x":40,"y":60,"width":300,"height":300})).unwrap();
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).wgpu().build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::Studio);
                app.ui.theme = crate::theme::ThemeKind::Studio;
                ready = true;
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(255, 0, 255))).show(ui, |_| {});
        },
        app,
    );
    h.run_steps(5);
    let area = h.ctx.memory(|m| m.area_rect(egui::Id::new("photocraft-panel-docking").with(("floating", &"swatches".to_string())))).unwrap();
    let image = h.render().unwrap();
    let ppp = image.width() as f32 / 1280.0;
    let strip = egui::Rect::from_min_size(area.min, vec2(area.width(), 40.0)).shrink(2.0);
    let magenta = (strip.left() as u32..strip.right() as u32)
        .flat_map(|x| (strip.top() as u32..strip.bottom() as u32).map(move |y| (x, y)))
        .filter(|&(x, y)| image.get_pixel((x as f32 * ppp) as u32, (y as f32 * ppp) as u32).0[..3] == [255, 0, 255])
        .count();
    assert_eq!(magenta, 0, "canvas shows through the floating group's title strip {strip:?}");
}

#[test]
fn escape_cancels_the_first_drag_without_replacing_the_legacy_workspace() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerMoved(source - vec2(90.0, 0.0)));
    h.run_steps(2);
    assert!(h.state().ui.docking.is_some());
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert!(h.state().ui.docking.is_none());
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().ui.docking.is_none());
}

#[test]
fn serialized_layout_actions_cover_geometry_reorder_accordion_and_atomic_rejection() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    app.run("window.panel.layout", json!({"action":{"ResizeSplit":{"path":[],"size":{"Ratio":0.35}}}})).unwrap();
    assert!(
        matches!(app.ui.docking.as_ref().unwrap().root, Some(Node::Split { size: craft_ui::layout::SplitSize::Ratio(value), .. }) if (value - 0.35).abs() < 0.001)
    );
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    app.run("window.panel.layout", json!({"action":{"MoveFloating":{"panel":"properties","rect":[44,66,360,450]}}})).unwrap();
    assert_eq!(
        app.ui.docking.as_ref().unwrap().floating.iter().find(|group| group.panels.iter().any(|p| p == "properties")).unwrap().rect,
        [44.0, 66.0, 360.0, 450.0]
    );
    app.run("window.panel.layout", json!({"action":{"Move":{"panel":"properties","anchor":"swatches","placement":{"Tab":{"before":"swatches"}}}}})).unwrap();
    assert_eq!(group_members_for_test(app.ui.docking.as_ref().unwrap(), "swatches"), ["properties", "swatches"]);
    let before = app.ui.docking.clone();
    let hidden = app.ui.docking_hidden.clone();
    for action in [
        json!({"MoveFloating":{"panel":"properties","rect":[0,0,-5,30]}}),
        json!({"ResizeSplit":{"path":[true,true,true],"size":{"Ratio":0.2}}}),
        json!({"Open":{"panel":"unknown","anchor":null}}),
        json!({"Move":{"panel":"properties","anchor":"swatches","placement":{"Tab":{"before":"unknown"}}}}),
    ] {
        assert!(app.run("window.panel.layout", json!({"action":action})).is_err());
        assert_eq!(app.ui.docking, before);
        assert_eq!(app.ui.docking_hidden, hidden);
    }
    app.ui.docking = Some(Layout {
        root: Some(Node::Stack { entries: vec![craft_ui::docking::StackEntry { panel: "layers".into(), open: true, height: Some(100.0) }] }),
        floating: Vec::new(),
    });
    app.run("window.panel.layout", json!({"action":{"SetStackOpen":{"panel":"layers","open":false}}})).unwrap();
    app.run("window.panel.layout", json!({"action":{"ResizeStack":{"panel":"layers","height":150}}})).unwrap();
    let Some(Node::Stack { entries }) = &app.ui.docking.as_ref().unwrap().root else { panic!("expected stack") };
    assert!(!entries[0].open);
    assert_eq!(entries[0].height, Some(150.0));
}

fn group_members_for_test(layout: &Layout<String>, panel: &str) -> Vec<String> {
    let mut pending: Vec<_> = layout.root.iter().collect();
    while let Some(node) = pending.pop() {
        match node {
            Node::Tabs { panels, .. } if panels.iter().any(|p| p == panel) => return panels.clone(),
            Node::Split { first, second, .. } => pending.extend([first.as_ref(), second.as_ref()]),
            _ => {}
        }
    }
    Vec::new()
}

#[test]
fn customization_retains_native_collapsed_groups_until_their_tab_is_activated() {
    let mut app = app();
    app.ui.dock.set_collapsed(Group::Color, true);
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let layout = app.ui.docking.as_ref().unwrap();
    assert!(collapsed_members(&app, layout).contains("swatches"));
    let saved = serde_json::to_value(&app.ui).unwrap();
    let restored: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert!(restored.dock.is_collapsed(Group::Color));
    app.run("window.panel.layout", json!({"action":{"Activate":{"panel":"swatches"}}})).unwrap();
    assert!(!app.ui.dock.is_collapsed(Group::Color));
}

#[test]
fn splitting_a_short_column_keeps_the_layers_footer_visible_and_usable() {
    let mut app = app();
    app.run("file.new", json!({"width":480,"height":360})).unwrap();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    app.run("window.panel.float", json!({"panel":"properties","x":30,"y":90,"width":300,"height":440})).unwrap();
    let mut h = harness(app, vec2(800.0, 800.0));
    let button = h.get_by_label("Create a new layer").rect();
    assert!(button.bottom() <= 800.0, "Layers footer is clipped below the viewport: {button:?}");
    let before = h.state().session.active().unwrap().doc.layers.len();
    let at = button.center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
}

#[test]
fn populated_canvas_keeps_a_real_opacity_edit_after_floating_and_redocking() {
    let _gpu = photocraft_testkit::gpu_lock();
    let mut app = app();
    app.ui.theme = crate::theme::ThemeKind::Studio;
    app.run("file.new", json!({"width":480,"height":360})).unwrap();
    app.run("layer.new.layer", json!({"name":"Docking sample"})).unwrap();
    app.run("select.rect", json!({"x":80,"y":70,"width":250,"height":160})).unwrap();
    app.run("edit.fill", json!({"color":"#5297bb"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("window.panel.float", json!({"panel":"properties","x":40,"y":140,"width":310,"height":490})).unwrap();
    let directory = std::env::var_os("CRAFT_UI_DOCKING_FIXTURES").map(std::path::PathBuf::from);
    let builder = Harness::builder().with_size(vec2(1280.0, 800.0));
    let builder = if directory.is_some() { builder.wgpu() } else { builder };
    let mut ready = false;
    let mut h = builder.build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::Studio);
                ready = true;
                return;
            }
            crate::panels::title_bar(app, ui);
            crate::panels::options_bar(app, ui);
            crate::panels::toolbar(app, ui);
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    h.input_mut().max_texture_side = Some(8192);
    h.run_steps(5);
    let floating = h.ctx.memory(|m| m.area_rect(egui::Id::new("photocraft-panel-docking").with(("floating", &"properties".to_string())))).unwrap();
    let mut sliders: Vec<_> = h.ctx.viewport(|vp| {
        vp.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, widgets)| widgets.iter())
            .filter(|widget| widget.sense.senses_drag() && widget.rect.height() == 18.0 && floating.contains_rect(widget.rect))
            .map(|widget| widget.rect)
            .collect()
    });
    sliders.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = sliders.first().expect("Properties opacity slider").center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    let opacity = h.state().session.active().unwrap().doc.layer(layer).unwrap().opacity;
    assert!((opacity - 0.5).abs() < 0.03, "actual opacity {opacity}");
    if let Some(directory) = &directory {
        std::fs::create_dir_all(directory).unwrap();
        h.render().unwrap().save(directory.join("photocraft-populated-opacity-floating.png")).unwrap();
    }
    let from = h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &"properties".to_string()))).unwrap().rect.center();
    let to = h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &"layers".to_string()))).unwrap().rect.center();
    drag(&mut h, from, to);
    assert!(h.state().ui.docking.as_ref().unwrap().floating.iter().all(|group| !group.panels.iter().any(|panel| panel == "properties")));
    assert_eq!(h.state().session.active().unwrap().doc.layer(layer).unwrap().opacity, opacity);
    if let Some(directory) = &directory {
        h.render().unwrap().save(directory.join("photocraft-populated-opacity-redocked.png")).unwrap();
    }
}

#[test]
fn changing_workspace_during_a_drag_cancels_the_stale_gesture() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    let destination = app.ui.docking.clone();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name":"Destination"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerMoved(source - vec2(90.0, 0.0)));
    h.run_steps(2);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.select", json!({"name":"Destination"})).unwrap();
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert_eq!(h.state().ui.docking, destination);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().ui.docking, destination);
}

#[test]
fn first_legacy_drag_workspace_switch_preserves_custom_and_legacy_destinations() {
    let layout_state = |app: &PhotocraftApp| json!({"panels":app.ui.panels,"dock":app.ui.dock,"tabs":app.ui.dock_tabs,"layout":app.ui.docking,"hidden":app.ui.docking_hidden,"collapsed":app.ui.docking_collapsed,"timeline":app.ui.timeline.open});
    for customized_destination in [true, false] {
        for cancellation in ["release", "escape", "focus-loss"] {
            let mut app = app();
            let ctx = egui::Context::default();
            if customized_destination {
                app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
                crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name":"Destination"})).unwrap();
                crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
            }
            let mut h = harness(app, vec2(1280.0, 800.0));
            let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
            h.event(Event::PointerMoved(source));
            h.run_steps(1);
            h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.run_steps(1);
            h.event(Event::PointerMoved(source - vec2(90.0, 0.0)));
            // One frame installs the first legacy drag; no shared dock has rendered yet.
            h.run_steps(1);
            assert!(h.state().ui.docking.is_some());
            let origin = egui::Id::new("photocraft-panel-docking").with("legacy-origin");
            assert!(h.ctx.data_mut(|data| data.get_temp::<LegacyDrag>(origin)).unwrap().layout.is_none());
            let ctx = h.ctx.clone();
            let command = if customized_destination { "window.workspace.select" } else { "window.workspace.painting" };
            crate::menus::invoke(h.state_mut(), &ctx, command, json!({"name":"Destination"})).unwrap();
            assert_eq!(h.state().ui.docking.is_some(), customized_destination);
            let destination = layout_state(h.state());
            let tabs = serde_json::to_value(h.state().ui.dock_tabs).unwrap();
            match cancellation {
                "escape" => h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }),
                "focus-loss" => h.event(Event::WindowFocused(false)),
                _ => {
                    h.event(Event::PointerButton { pos: source - vec2(90.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE })
                }
            }
            h.run_steps(1);
            assert_eq!(layout_state(h.state()), destination, "{customized_destination}/{cancellation}");
            if cancellation == "focus-loss" {
                let at = source - vec2(90.0, 0.0);
                h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
                h.run_steps(1);
                assert_eq!(layout_state(h.state()), destination, "secondary press must not resume the stale primary drag");
                h.event(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
                h.run_steps(1);
            }
            assert_eq!(serde_json::to_value(h.state().ui.dock_tabs).unwrap(), tabs);
            assert!(h.ctx.data_mut(|data| data.get_temp::<LegacyDrag>(origin)).is_none(), "stale origin: {customized_destination}/{cancellation}");
            h.event(Event::WindowFocused(true));
            h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(2);
            assert_eq!(layout_state(h.state()), destination);
        }
    }
}

fn control_set(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Value {
    let (request, _) = crate::control::ControlRequest::new("ui.set", params);
    match crate::control::handle(app, ctx, &request) {
        crate::control::Outcome::Done(value) => value,
        _ => panic!("ui.set must reply synchronously"),
    }
}

#[test]
fn current_main_hidden_tabs_migrate_by_name_and_selected_identity() {
    for theme in [crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::Studio] {
        let mut app = app();
        app.ui.theme = theme;
        let pro = matches!(theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
        let color = Group::Color.tabs(pro).iter().position(|name| *name == "Color").unwrap();
        let swatches = Group::Color.tabs(pro).iter().position(|name| *name == "Swatches").unwrap();
        app.ui.dock.hide_tab(Group::Color, swatches, pro);
        app.ui.dock.hide_tab(Group::Layers, 2, pro);
        app.ui.dock_tabs.color = color;
        let saved = json!({"panels":app.ui.panels,"dock":app.ui.dock,"dockTabs":app.ui.dock_tabs});
        assert_eq!(saved["dock"]["hidden_tabs"]["layers"], json!(["Paths"]));
        let mut restored = self::app();
        restored.ui.theme = theme;
        crate::dock::apply(&mut restored, &saved);
        restored.run("window.panel.float", json!({"panel":"properties"})).unwrap();
        let layout = restored.ui.docking.as_ref().unwrap();
        assert!(!layout.contains(&"swatches".into()) && !layout.contains(&"paths".into()));
        assert!(panel_active(layout, "color"));
        assert!(restored.ui.docking_hidden.contains_key("swatches") && restored.ui.docking_hidden.contains_key("paths"));
        restored.run("window.panel.swatches", json!({"show":true})).unwrap();
        assert!(panel_active(restored.ui.docking.as_ref().unwrap(), "swatches"));
    }
}

#[test]
fn legacy_control_updates_authoritative_custom_visibility_and_selection_atomically() {
    let mut app = app();
    app.ui.theme = crate::theme::ThemeKind::ProMedium;
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(control_set(&mut app, &ctx, json!({"dockTabs":{"layers":1}}))["ok"], true);
    assert!(panel_active(app.ui.docking.as_ref().unwrap(), "channels"));
    assert_eq!(control_set(&mut app, &ctx, json!({"panels":{"layers":false}}))["ok"], true);
    assert!(PANELS.iter().filter(|p| p.2 == Group::Layers).all(|p| !app.ui.docking.as_ref().unwrap().contains(&p.0.into())));
    assert!(!app.ui.panels.layers);
    assert_eq!(control_set(&mut app, &ctx, json!({"panels":{"layers":true},"dockTabs":{"layers":2}}))["ok"], true);
    assert!(panel_active(app.ui.docking.as_ref().unwrap(), "paths"));
    let before = serde_json::to_value(&app.ui).unwrap();
    for request in [
        json!({"panels":{"color":false},"dockTabs":{"layers":999}}),
        json!({"tool":"brush","dock":{"order":["layers","color"]}}),
        json!({"panels":{"layers":false},"dockTabs":{"layers":1}}),
    ] {
        assert_eq!(control_set(&mut app, &ctx, request)["ok"], false);
        assert_eq!(serde_json::to_value(&app.ui).unwrap(), before);
    }
}

#[test]
fn legacy_dock_width_is_consumed_by_custom_renderer_and_clamped_to_existing_range() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let ctx = h.ctx.clone();
    // A 1280-point window: the dock may take all but 400 points (#7896), so 700 is no longer
    // cut to 520.
    for (requested, expected) in [(410.0, 410.0), (700.0, 700.0), (900.0, 880.0), (20.0, 250.0)] {
        assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth":requested}))["ok"], true);
        h.run_steps(2);
        let state = egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new("shared-panel-dock")).unwrap();
        assert!((state.outer_rect.width() - expected).abs() < 1.0, "{} / {expected}", state.outer_rect.width());
        assert!(h.ctx.data(|data| data.get_temp::<f32>(crate::panels::dock_width_id())).is_none());
    }
}

#[test]
fn explicit_reveal_activates_existing_tab_and_expands_custom_group() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"properties","anchor":"layers"})).unwrap();
    app.run("window.panel.group", json!({"panel":"layers","operation":"collapse"})).unwrap();
    app.run("window.panel.layers", json!({"show":true})).unwrap();
    assert!(panel_active(app.ui.docking.as_ref().unwrap(), "layers"));
    assert!(!collapsed_members(&app, app.ui.docking.as_ref().unwrap()).contains("layers"));
    app.run("window.panel.layers", json!({"show":true})).unwrap();
    assert!(app.ui.docking.as_ref().unwrap().contains(&"layers".into()));
    let before = app.ui.docking.clone();
    assert!(app.run("window.panel.layers", json!({"show":"yes"})).is_err());
    assert_eq!(app.ui.docking, before);
}

// #2455 in a custom workspace: the pro Color field fills a tall floating group, not 120 pt.
#[test]
fn pro_color_field_fills_a_tall_custom_group() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"color","x":40,"y":60,"width":320,"height":560})).unwrap();
    assert!(app.ui.docking.is_some());
    let h = harness(app, vec2(1280.0, 800.0));
    // The SV field and the 14 pt hue strip beside it are draggable, equally tall and 8 pt apart.
    let drags: Vec<egui::Rect> =
        h.ctx.viewport(|v| v.this_pass.widgets.layers().flat_map(|(_, w)| w).filter(|w| w.sense.senses_drag()).map(|w| w.rect).collect());
    let field = drags
        .iter()
        .find(|f| drags.iter().any(|s| s.width() == 14.0 && s.top() == f.top() && s.height() == f.height() && (s.left() - f.right() - 8.0).abs() < 0.5))
        .expect("Color field with its hue strip");
    assert!(field.height() > 300.0, "field stays short in a 560 pt group: {field:?}");
}

// #4028: in a custom workspace too, a new adjustment layer brings Properties to the front.
#[test]
fn a_new_adjustment_layer_activates_properties_in_a_custom_workspace() {
    let mut app = app();
    app.run("window.panel.activate", json!({"panel":"adjustments"})).unwrap();
    assert!(panel_active(app.ui.docking.as_ref().unwrap(), "adjustments"));
    app.run("file.new", json!({"width":64,"height":64})).unwrap();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    assert!(panel_active(app.ui.docking.as_ref().unwrap(), "properties"));
}

#[test]
fn custom_group_commands_preserve_members_and_workspace_timeline() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"properties","anchor":"layers"})).unwrap();
    let members = group_members(app.ui.docking.as_ref().unwrap(), "layers");
    app.run("window.panel.group", json!({"panel":"layers","operation":"up"})).unwrap();
    assert_eq!(group_members(app.ui.docking.as_ref().unwrap(), "layers"), members);
    app.ui.timeline.open = true;
    app.run("window.panel.group", json!({"panel":"layers","operation":"collapse"})).unwrap();
    let saved = json!({"panels":app.ui.panels,"dock":app.ui.dock,"dockTabs":app.ui.dock_tabs,"docking":app.ui.docking,"dockingHidden":app.ui.docking_hidden,"dockingCollapsed":app.ui.docking_collapsed,"timelineOpen":true});
    let mut restored = self::app();
    crate::dock::apply(&mut restored, &saved);
    assert_eq!(restored.ui.docking, app.ui.docking);
    assert!(restored.ui.timeline.open);
    assert!(collapsed_members(&restored, restored.ui.docking.as_ref().unwrap()).contains("layers"));
    app.run("window.panel.group", json!({"panel":"layers","operation":"close"})).unwrap();
    assert!(members.iter().all(|id| !app.ui.docking.as_ref().unwrap().contains(id)));
    assert!(members.iter().all(|id| app.ui.docking_hidden.contains_key(id)));
    app.session.prefs.edit(|p| p.workspace_locked = true);
    let before = serde_json::to_value(&app.ui).unwrap();
    assert!(app.run("window.panel.group", json!({"panel":"color","operation":"close"})).is_err());
    assert_eq!(serde_json::to_value(&app.ui).unwrap(), before);
    restored.ui.timeline.playing = true;
    crate::dock::apply(&mut restored, &json!({"dock":{}}));
    assert!(!restored.ui.timeline.open && !restored.ui.timeline.playing);
}

#[test]
fn customized_panel_menus_and_rail_remain_reachable_after_float_and_restore() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"swatches","x":20,"y":40})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    assert!(h.query_all_by_label("Panel menu").count() > 0);
    assert!(h.query_all_by_label("Color & Swatches").count() > 0);
    let floating = h.ctx.memory(|memory| memory.area_rect(egui::Id::new("photocraft-panel-docking").with(("floating", &"swatches".to_string())))).unwrap();
    let first = h.query_all_by_label("Panel menu").find(|node| floating.contains_rect(node.rect())).unwrap().rect().center();
    h.hover_at(first);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: first, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    assert!(h.query_all_by_label("Close Tab Group").count() > 0);
    assert!(h.query_all_by_label("New Swatch…").count() > 0);
    // Panel-specific menu body is shared by both its context and group menu seams.
    let mut ready = false;
    let mut menu = Harness::builder().build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
            }
            panel_menu(app, ui, "swatches");
        },
        self::app(),
    );
    menu.run_steps(2);
    assert!(menu.query_all_by_label("New Swatch…").count() > 0);
}

/// A floated Color panel's menu still picks its mode (#6000): the shared docking renderer has
/// its own panel menu, separate from the legacy dock's.
#[test]
fn floated_color_panel_menu_picks_the_mode() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"color","x":20,"y":40})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let floating = h.ctx.memory(|memory| memory.area_rect(egui::Id::new("photocraft-panel-docking").with(("floating", &"color".to_string())))).unwrap();
    let button = h.query_all_by_label("Panel menu").find(|node| floating.contains_rect(node.rect())).unwrap().rect().center();
    h.hover_at(button);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: button, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    h.get_by_label("   RGB Sliders").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.color_panel.mode, crate::state::ColorPanelMode::RgbSliders);
}

#[test]
fn actions_panel_menu_includes_load_and_save_actions() {
    let mut ready = false;
    let mut menu = Harness::builder().build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
            }
            panel_menu(app, ui, "actions");
        },
        self::app(),
    );
    menu.run_steps(2);
    assert!(menu.query_all_by_label("Load Actions…").count() > 0);
    assert!(menu.query_all_by_label("Save Actions…").count() > 0);
    assert!(menu.query_all_by_label("New Action…").count() > 0);
}

#[test]
fn custom_workspace_saves_shared_layout_and_timeline_without_embedding_flags() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    app.ui.timeline.open = true;
    app.ui.panels.rail = false;
    app.ui.panels.menu_bar = false;
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name":"Docked Motion"})).unwrap();
    let saved = app.session.prefs().workspaces.get("Docked Motion").unwrap().clone();
    assert!(saved["docking"].is_object() && saved["timelineOpen"] == true);
    assert!(saved["panels"].get("rail").is_none() && saved["panels"].get("menu_bar").is_none());
    let layout = app.ui.docking.clone();
    app.ui.workspace = "Motion".into();
    crate::menus::apply_workspace(&mut app);
    assert!(app.ui.docking.is_none() && app.ui.timeline.open);
    crate::dock::apply(&mut app, &saved);
    assert_eq!(app.ui.docking, layout);
    assert!(app.ui.timeline.open);
}

#[test]
fn closing_and_reopening_a_whole_group_retains_its_return_position_and_tab_order() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let before = app.ui.docking.clone().unwrap();
    let members = group_members(&before, "layers");
    app.run("window.panel.group", json!({"panel":"layers","operation":"close"})).unwrap();
    for panel in &members {
        app.run("window.panel.activate", json!({"panel":panel})).unwrap();
    }
    let after = app.ui.docking.as_ref().unwrap();
    assert_eq!(group_members(after, "layers"), members);
    assert_eq!(root_groups(after), root_groups(&before));
    assert!(valid(after));
}

#[test]
fn floating_group_grip_redocks_all_tabs_as_one_atomic_move() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"swatches","x":30,"y":60})).unwrap();
    app.run("window.panel.move", json!({"panel":"patterns","anchor":"swatches"})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let from = h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("move-group", Some(&"swatches".to_string())))).unwrap().rect.center();
    let to = h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &"layers".to_string()))).unwrap().rect.center();
    drag(&mut h, from, to);
    let layout = h.state().ui.docking.as_ref().unwrap();
    assert!(layout.floating.iter().all(|group| !group.panels.iter().any(|id| id == "swatches" || id == "patterns")));
    let members = group_members(layout, "layers");
    let swatches = members.iter().position(|id| id == "swatches").unwrap();
    assert_eq!(members.get(swatches + 1).map(String::as_str), Some("patterns"));
    assert!(valid(layout));
    let before = h.state().ui.docking.clone();
    assert!(h.state_mut().run("window.panel.group", json!({"panel":"swatches","operation":"move","anchor":"patterns"})).is_err());
    assert_eq!(h.state().ui.docking, before);
}

#[test]
fn mixed_theme_and_legacy_selection_projects_the_selected_panel_in_the_new_theme() {
    let mut app = app();
    app.ui.theme = crate::theme::ThemeKind::ProMedium;
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(control_set(&mut app, &ctx, json!({"theme":"studio","dockTabs":{"color":0}}))["ok"], true);
    assert!(panel_active(app.ui.docking.as_ref().unwrap(), "swatches"));
    assert_eq!(app.ui.dock_tabs.color, 0);
    assert_eq!(control_set(&mut app, &ctx, json!({"theme":"proMedium","dockTabs":{"color":0}}))["ok"], true);
    assert!(panel_active(app.ui.docking.as_ref().unwrap(), "color"));
    assert_eq!(app.ui.dock_tabs.color, 0);
}

#[test]
fn native_tab_reorder_stays_native_until_drag_crosses_the_strip() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    let target = h.query_all_by_label("Channels").find(|node| node.rect().height() <= 45.0).unwrap().rect().right_center() - vec2(1.0, 0.0);
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    let near_edge = egui::pos2(target.x, source.y - 21.0);
    h.event(Event::PointerMoved(near_edge));
    h.run_steps(1);
    assert!(h.state().ui.docking.is_none(), "native near-edge reorder tolerance must not promote to custom docking");
    h.event(Event::PointerMoved(target));
    h.run_steps(1);
    assert!(h.state().ui.docking.is_none(), "native horizontal reorder must not promote to custom docking");
    h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().ui.docking.is_none());
    let tabs = h.state().ui.dock.visible_tabs(Group::Layers, false);
    let layers = tabs.iter().position(|(_, label)| *label == "Layers").unwrap();
    let channels = tabs.iter().position(|(_, label)| *label == "Channels").unwrap();
    assert!(channels < layers, "upstream native reorder remains effective");
}

fn floating_rect(app: &PhotocraftApp, panel: &str) -> Option<[f32; 4]> {
    app.ui.docking.as_ref()?.floating.iter().find(|g| g.panels.iter().any(|p| p == panel)).map(|g| g.rect)
}

#[test]
fn window_menu_opens_a_hidden_panel_floating_and_toggles_it_closed() {
    let mut app = app();
    assert!(!app.ui.panels.navigator, "Navigator starts hidden");
    app.run("window.panel.navigator", json!({})).unwrap();
    assert_eq!(floating_rect(&app, "navigator"), Some(FLOAT_ORIGIN));
    assert!(app.ui.panels.navigator);
    assert!(valid(app.ui.docking.as_ref().unwrap()));
    // The Window item is still a toggle: a second click closes the floating window.
    app.run("window.panel.navigator", json!({})).unwrap();
    assert!(!app.ui.docking.as_ref().unwrap().contains(&"navigator".into()));
    // The legacy `window.toggle.*` ids float the same way.
    app.run("window.toggle.history", json!({})).unwrap();
    assert!(floating_rect(&app, "history").is_some());
}

#[test]
fn a_panel_closed_while_floating_reopens_where_it_was_left() {
    let mut app = app();
    app.run("window.panel.histogram", json!({})).unwrap();
    let moved = [90.0, 220.0, 360.0, 300.0];
    app.run("window.panel.layout", json!({"action": {"MoveFloating": {"panel": "histogram", "rect": moved}}})).unwrap();
    assert_eq!(floating_rect(&app, "histogram"), Some(moved));
    app.run("window.panel.histogram", json!({})).unwrap();
    assert_eq!(floating_rect(&app, "histogram"), None);
    app.run("window.panel.histogram", json!({})).unwrap();
    assert_eq!(floating_rect(&app, "histogram"), Some(moved));
}

#[test]
fn panels_opened_one_after_another_open_side_by_side_instead_of_stacking() {
    let mut app = app();
    app.last_canvas_rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0));
    app.run("window.panel.navigator", json!({})).unwrap();
    app.run("window.panel.info", json!({})).unwrap();
    let (a, b) = (floating_rect(&app, "navigator").unwrap(), floating_rect(&app, "info").unwrap());
    assert_eq!(a, FLOAT_ORIGIN);
    assert_eq!(b, [a[0] + a[2] + FLOAT_GAP, a[1], a[2], a[3]]);
}

#[test]
fn a_panel_opened_beside_a_floating_one_leaves_it_visible() {
    // The known issue: with Navigator floating, Window › Histogram opened a window 28 points
    // off Navigator's, covering nearly all of it, so Navigator seemed to disappear.
    let mut app = app();
    app.last_canvas_rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0));
    let mut h = harness(app, vec2(1400.0, 900.0));
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.navigator", json!({})).unwrap();
    h.run_steps(4);
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.histogram", json!({})).unwrap();
    h.run_steps(4);
    let [x, y, w, hh] = floating_rect(h.state(), "navigator").unwrap();
    let area = egui::Id::new("photocraft-panel-docking");
    let navigator = egui::LayerId::new(egui::Order::Middle, area.with(("floating", "navigator")));
    assert_eq!(h.ctx.layer_id_at(egui::pos2(x + w / 2.0, y + hh / 2.0)), Some(navigator), "Navigator stays on top of its own spot");
    let rect = |r: [f32; 4]| egui::Rect::from_min_size(egui::pos2(r[0], r[1]), vec2(r[2], r[3]));
    assert!(!rect([x, y, w, hh]).intersects(rect(floating_rect(h.state(), "histogram").unwrap())));
}

#[test]
fn with_no_room_beside_them_new_panels_cascade_onto_a_free_corner() {
    let mut app = app();
    // A canvas area that fits only one window: the rest cascade, each from its own corner.
    app.last_canvas_rect = egui::Rect::from_min_size(egui::pos2(380.0, 0.0), vec2(340.0, 600.0));
    for panel in ["navigator", "info", "histogram"] {
        app.run(&format!("window.panel.{panel}"), json!({})).unwrap();
    }
    let corners: Vec<[f32; 2]> = ["navigator", "info", "histogram"].iter().map(|p| floating_rect(&app, p).unwrap()).map(|r| [r[0], r[1]]).collect();
    assert_eq!(corners, [[400.0, 140.0], [428.0, 168.0], [456.0, 196.0]]);
    // Closing the middle one frees its corner for the next panel.
    app.run("window.panel.info", json!({})).unwrap();
    app.run("window.panel.history", json!({})).unwrap();
    assert_eq!(floating_rect(&app, "history").map(|r| [r[0], r[1]]), Some([428.0, 168.0]));
}

#[test]
fn saved_separate_windows_with_unusable_geometry_are_forgotten_not_opened() {
    let mut app = app();
    let saved = json!({
        "dockingWindows": {
            "navigator": [10.0, 20.0, 300.0, 400.0],
            "info": [0.0, 0.0, 1.0e30, 400.0],
            "histogram": [-5.0e9, 0.0, 300.0, 400.0],
            "layers": [0.0, 0.0, 0.0, 0.0],
            "notAPanel": [0.0, 0.0, 300.0, 400.0],
            "history": null
        },
        "railPanels": ["color", "color", "nope", "layers"]
    });
    crate::dock::apply(&mut app, &saved);
    let windows = &app.ui.docking_windows;
    assert_eq!(windows.get("navigator"), Some(&Some([10.0, 20.0, 300.0, 400.0])));
    for panel in ["info", "histogram", "layers", "history"] {
        assert_eq!(windows.get(panel), Some(&None), "{panel} opens over the app window instead");
    }
    assert!(!windows.contains_key("notAPanel"));
    assert_eq!(app.ui.rail_panels, ["color", "layers"]);
    // The command refuses a window the GPU surface couldn't back.
    let err = app.run("window.panel.detach", json!({"panel": "navigator", "x": 0, "y": 0, "width": 100000, "height": 300})).unwrap_err();
    assert!(err.contains("at most"), "{err}");
}

#[test]
fn a_showing_panel_setting_off_or_locked_workspace_keep_dock_behaviour() {
    // Through the menu, as a click would: the native dock's Window items are aliases there.
    let ctx = egui::Context::default();
    // Already showing: Window › Layers keeps toggling it in the dock, never tears it off.
    let mut app = app();
    crate::menus::invoke(&mut app, &ctx, "window.panel.layers", json!({})).unwrap();
    assert!(floating_rect(&app, "layers").is_none());
    // Setting off: a hidden panel opens in the dock, as in Photoshop.
    let mut app = self::app();
    app.session.prefs.edit(|p| p.window_panels_docked = true);
    crate::menus::invoke(&mut app, &ctx, "window.panel.navigator", json!({})).unwrap();
    assert!(app.ui.panels.navigator, "opens in the native dock");
    assert!(floating_rect(&app, "navigator").is_none());
    // Locked workspace: nothing may tear off.
    let mut app = self::app();
    app.session.prefs.edit(|p| p.workspace_locked = true);
    let _ = crate::menus::invoke(&mut app, &ctx, "window.panel.navigator", json!({}));
    assert!(floating_rect(&app, "navigator").is_none());
}

#[test]
fn explicit_hide_and_bad_params_do_not_float_anything() {
    let mut app = app();
    for show in [json!(false), json!("yes"), json!(1)] {
        let _ = app.run("window.panel.navigator", json!({ "show": show }));
        assert!(floating_rect(&app, "navigator").is_none(), "show: {show}");
    }
}

#[test]
fn open_panels_floating_menu_item_toggles_and_shows_its_checkmark() {
    let mut app = app();
    let ctx = egui::Context::default();
    assert_eq!(crate::workspace_ui::checked(&app, "window.workspace.openPanelsFloating"), Some(true));
    crate::menus::invoke(&mut app, &ctx, "window.workspace.openPanelsFloating", json!({})).unwrap();
    assert!(app.session.prefs().window_panels_docked);
    assert_eq!(crate::workspace_ui::checked(&app, "window.workspace.openPanelsFloating"), Some(false));
    crate::menus::invoke(&mut app, &ctx, "window.workspace.openPanelsFloating", json!({"on": true})).unwrap();
    assert!(!app.session.prefs().window_panels_docked);
    assert!(crate::menus::invoke(&mut app, &ctx, "window.workspace.openPanelsFloating", json!({"on": 3})).is_err());
    assert!(!app.session.prefs().window_panels_docked);
}

/// Splits in the right dock, left/right ones being side-by-side columns.
fn has_split(layout: &Layout<String>, axis: SplitAxis) -> bool {
    let mut pending: Vec<_> = layout.root.iter().collect();
    while let Some(node) = pending.pop() {
        if let Node::Split { axis: a, first, second, .. } = node {
            if *a == axis {
                return true;
            }
            pending.extend([first.as_ref(), second.as_ref()]);
        }
    }
    false
}

#[test]
fn a_window_menu_panel_floats_then_drags_into_a_new_dock_column_and_back_out() {
    let shots = std::env::var_os("PHOTOCRAFT_FLOAT_SHOTS").map(std::path::PathBuf::from);
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    let mut ready = false;
    let mut builder = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0);
    if shots.is_some() {
        builder = builder.wgpu();
    }
    let mut h = builder.build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                app.ui.theme = crate::theme::ThemeKind::ProMedium;
                ready = true;
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    h.run_steps(4);
    let shot = |h: &mut Harness<'static, PhotocraftApp>, name: &str| {
        if let Some(dir) = &shots {
            std::fs::create_dir_all(dir).unwrap();
            h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
        }
    };
    shot(&mut h, "1-start");
    // Window › Navigator: a floating window.
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.navigator", json!({})).unwrap();
    h.run_steps(4);
    assert!(floating_rect(h.state(), "navigator").is_some());
    shot(&mut h, "2-navigator-floating");
    let dock = |h: &Harness<'static, PhotocraftApp>| {
        h.ctx.data_mut(|d| d.get_temp::<(usize, f32)>(egui::Id::new("photocraft-panel-docking").with("columns"))).unwrap()
    };
    let (one, narrow) = dock(&h);
    assert_eq!(one, 1);
    // Drag its tab onto the left edge of the Layers group: a new column beside Layers.
    let tab = |h: &Harness<'static, PhotocraftApp>, panel: &str| {
        h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &panel.to_string()))).unwrap().rect
    };
    let layers = tab(&h, "layers");
    let from = tab(&h, "navigator").center();
    drag(&mut h, from, egui::pos2(layers.left() + 6.0, layers.bottom() + 140.0));
    let layout = h.state().ui.docking.clone().unwrap();
    assert!(floating_rect(h.state(), "navigator").is_none(), "{layout:?}");
    assert!(has_split(&layout, SplitAxis::Horizontal), "docked as a column: {layout:?}");
    h.run_steps(3);
    let (two, wide) = dock(&h);
    assert_eq!(two, 2);
    assert!(wide >= narrow + COLUMN_WIDTH - 1.0, "the dock grows by a column: {narrow} -> {wide}");
    assert!(valid(&layout));
    shot(&mut h, "3-navigator-docked-as-column");
    // And straight back out: drag the docked tab over the canvas to float it again.
    let from = tab(&h, "navigator").center();
    drag(&mut h, from, egui::pos2(300.0, 300.0));
    assert!(floating_rect(h.state(), "navigator").is_some());
    assert!(!has_split(&h.state().ui.docking.clone().unwrap(), SplitAxis::Horizontal));
    h.run_steps(3);
    let (back, width) = dock(&h);
    assert_eq!(back, 1);
    assert!((width - narrow).abs() < 1.0, "and shrinks back: {width} vs {narrow}");
    shot(&mut h, "4-navigator-undocked-again");
}

#[test]
fn move_to_separate_window_floats_a_docked_panel_and_return_brings_it_back() {
    let mut app = app();
    // A docked panel floats first, then gets its own window.
    app.run("window.panel.detach", json!({"panel": "layers"})).unwrap();
    assert!(floating_rect(&app, "layers").is_some());
    assert_eq!(app.ui.docking_windows.get("layers"), Some(&None));
    // Its tab mates share the one window: no second entry.
    app.run("window.panel.move", json!({"panel": "channels", "anchor": "layers"})).unwrap();
    app.run("window.panel.detach", json!({"panel": "channels", "x": 1900, "y": 40, "width": 320, "height": 500})).unwrap();
    assert!(!app.ui.docking_windows.contains_key("layers"));
    assert_eq!(app.ui.docking_windows.get("channels"), Some(&Some([1900.0, 40.0, 320.0, 500.0])));
    app.run("window.panel.attach", json!({"panel": "layers"})).unwrap();
    assert!(app.ui.docking_windows.is_empty());
    assert!(floating_rect(&app, "layers").is_some(), "back floating inside PhotoCraft");
    // Survives a save and restore of the UI state.
    app.run("window.panel.detach", json!({"panel": "layers", "x": -1500, "y": 200, "width": 300, "height": 420})).unwrap();
    let restored: crate::state::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert_eq!(restored.docking_windows, app.ui.docking_windows);
}

#[test]
fn a_separate_window_entry_waits_while_closed_and_is_dropped_once_docked() {
    let mut app = app();
    app.run("window.panel.navigator", json!({})).unwrap();
    app.run("window.panel.detach", json!({"panel": "navigator"})).unwrap();
    // Closed (the window's close button): remembered, so Window › Navigator brings the window back.
    app.run("window.panel.group", json!({"panel": "navigator", "operation": "close"})).unwrap();
    let layout = app.ui.docking.clone().unwrap();
    assert_eq!(separate_windows(&mut app, &layout), Vec::<String>::new());
    assert!(app.ui.docking_windows.contains_key("navigator"));
    app.run("window.panel.navigator", json!({})).unwrap();
    let layout = app.ui.docking.clone().unwrap();
    assert_eq!(separate_windows(&mut app, &layout), vec!["navigator".to_string()]);
    // Docked: no longer a window.
    app.run("window.panel.dock", json!({"panel": "navigator"})).unwrap();
    let layout = app.ui.docking.clone().unwrap();
    assert!(separate_windows(&mut app, &layout).is_empty());
    assert!(app.ui.docking_windows.is_empty());
}

#[test]
fn separate_window_commands_reject_bad_input_and_a_locked_workspace() {
    let mut app = app();
    for params in [
        json!({}),
        json!({"panel": "unknown"}),
        json!({"panel": "layers", "x": 10}),
        json!({"panel": "layers", "x": 0, "y": 0, "width": 0, "height": 100}),
        json!({"panel": "layers", "x": "a", "y": 0, "width": 100, "height": 100}),
        json!({"panel": "layers", "x": 1e12, "y": 0, "width": 100, "height": 100}),
    ] {
        assert!(app.run("window.panel.detach", params.clone()).is_err(), "{params}");
    }
    assert!(app.ui.docking_windows.is_empty());
    app.session.prefs.edit(|p| p.workspace_locked = true);
    assert!(app.run("window.panel.detach", json!({"panel": "layers"})).is_err());
    assert!(app.run("window.panel.attach", json!({"panel": "layers"})).is_err());
}

#[test]
fn a_separate_window_draws_its_panel_outside_the_in_app_layout() {
    let mut app = app();
    app.run("window.panel.navigator", json!({})).unwrap();
    app.run("window.panel.detach", json!({"panel": "navigator"})).unwrap();
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                // As on the desktop: real windows. With no native backend here, egui draws the
                // window inside this one, through the same code.
                ui.ctx().set_embed_viewports(false);
                ready = true;
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    h.run_steps(4);
    assert!(h.query_by_label_contains("Navigator — PhotoCraft").is_some(), "the separate window, titled after its panels");
    assert!(navigator_left_of_dock(&h), "showing the Navigator panel");
    // Closing it closes the panel but remembers the window.
    h.state_mut().run("window.panel.group", json!({"panel": "navigator", "operation": "close"})).unwrap();
    h.run_steps(2);
    assert!(h.query_by_label_contains("Navigator — PhotoCraft").is_none());
    assert!(h.state().ui.docking_windows.contains_key("navigator"));
}

#[test]
fn dragging_a_divider_towards_collapsed_groups_expands_one_so_the_group_above_can_shrink() {
    let shots = std::env::var_os("PHOTOCRAFT_FLOAT_SHOTS").map(std::path::PathBuf::from);
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    // A tall Properties above three collapsed groups: before the fix it had to fill the column.
    let layout: Layout<String> = serde_json::from_value(json!({"floating":[],"root":{"Split":{"axis":"Vertical","size":{"FixedFirst":190.0},
        "first":{"Tabs":{"panels":["color","patterns","adjustments"],"active":0}},
        "second":{"Split":{"axis":"Vertical","size":{"FixedFirst":718.0},"first":{"Tabs":{"panels":["properties","swatches","gradients"],"active":0}},
        "second":{"Split":{"axis":"Vertical","size":{"FixedFirst":210.0},"first":{"Tabs":{"panels":["navigator"],"active":0}},
        "second":{"Split":{"axis":"Vertical","size":{"FixedFirst":200.0},"first":{"Tabs":{"panels":["history","actions","layerComps"],"active":2}},
        "second":{"Tabs":{"panels":["layers","channels","paths"],"active":0}}}}}}}}}}}))
    .unwrap();
    app.ui.docking = Some(layout);
    app.ui.docking_collapsed = ["layers", "channels", "paths", "navigator"].map(String::from).to_vec();
    // History collapsed through the native dock flag, as a remembered workspace can leave it.
    app.ui.dock.set_collapsed(Group::History, true);
    let mut ready = false;
    let mut builder = Harness::builder().with_size(vec2(1600.0, 1100.0)).with_step_dt(1.0 / 60.0);
    if shots.is_some() {
        builder = builder.wgpu();
    }
    let mut h = builder.build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                app.ui.theme = crate::theme::ThemeKind::ProMedium;
                ready = true;
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    h.run_steps(4);
    let tab = |h: &Harness<'static, PhotocraftApp>, panel: &str| {
        h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &panel.to_string()))).unwrap().rect
    };
    let (props, nav) = (tab(&h, "properties"), tab(&h, "navigator"));
    let tall = nav.top() - props.top();
    let from = egui::pos2(nav.center().x, nav.top() - 2.0);
    drag(&mut h, from, from - vec2(0.0, 400.0));
    h.run_steps(3);
    let collapsed = collapsed_members(h.state(), h.state().ui.docking.as_ref().unwrap());
    assert!(!collapsed.contains("navigator"), "the group under the divider expands");
    let short = tab(&h, "navigator").top() - tab(&h, "properties").top();
    assert!(short < tall - 200.0, "Properties shrinks: {tall} -> {short}");
    if let Some(dir) = &shots {
        h.render().unwrap().save(dir.join("s-after-shrink.png")).unwrap();
    }
}

#[test]
fn icon_bar_commands_dock_open_and_return_panels() {
    let mut app = app();
    let ctx = egui::Context::default();
    // A docked group goes to the bar whole; its old spot is remembered.
    app.run("window.panel.toRail", json!({"panel": "layers", "group": true})).unwrap();
    assert_eq!(app.ui.rail_panels, ["layers", "channels", "paths"]);
    let layout = app.ui.docking.clone().unwrap();
    assert!(!layout.contains(&"layers".into()) && !layout.contains(&"channels".into()));
    assert!(rail_replaces(&app, Group::Layers), "the rail's own Layers button steps aside");
    // Window › Layers opens its flyout instead of pulling it off the bar, and is checked.
    crate::menus::invoke(&mut app, &ctx, "window.panel.layers", json!({})).unwrap();
    assert_eq!(app.ui.rail_open.as_deref(), Some("layers"));
    assert_eq!(checked(&app, "window.panel.layers"), Some(true));
    crate::menus::invoke(&mut app, &ctx, "window.panel.layers", json!({})).unwrap();
    assert_eq!(app.ui.rail_open, None);
    // Back to the dock where it was, to a floating window, or just off the bar.
    app.run("window.panel.fromRail", json!({"panel": "layers", "to": "dock"})).unwrap();
    assert!(app.ui.docking.as_ref().unwrap().contains(&"layers".into()));
    app.run("window.panel.fromRail", json!({"panel": "channels", "to": "float"})).unwrap();
    assert!(floating_rect(&app, "channels").is_some());
    app.run("window.panel.fromRail", json!({"panel": "paths", "to": "none"})).unwrap();
    assert!(app.ui.rail_panels.is_empty());
    assert!(!app.ui.docking.as_ref().unwrap().contains(&"paths".into()));
}

#[test]
fn a_panel_brought_back_another_way_leaves_the_bar_and_the_bar_is_saved() {
    let mut app = app();
    app.run("window.panel.navigator", json!({})).unwrap();
    app.run("window.panel.detach", json!({"panel": "navigator"})).unwrap();
    app.run("window.panel.toRail", json!({"panel": "navigator"})).unwrap();
    assert_eq!(app.ui.rail_panels, ["navigator"]);
    assert!(app.ui.docking_windows.is_empty(), "no longer a separate window");
    // Saved with the workspace and restored.
    let saved = serde_json::to_value(&app.ui).unwrap();
    let restored: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.rail_panels, ["navigator"]);
    // Dock it by command: it can't be in both places.
    app.run("window.panel.dock", json!({"panel": "navigator"})).unwrap();
    assert!(app.ui.rail_panels.is_empty());
}

#[test]
fn icon_bar_commands_reject_bad_input_and_a_locked_workspace() {
    let mut app = app();
    for (id, params) in [
        ("window.panel.toRail", json!({})),
        ("window.panel.toRail", json!({"panel": "nope"})),
        ("window.panel.toRail", json!({"panel": "layers", "group": "yes"})),
        ("window.panel.fromRail", json!({"panel": "layers"})),
        ("window.panel.fromRail", json!({"panel": "layers", "to": "moon"})),
    ] {
        assert!(app.run(id, params.clone()).is_err(), "{id} {params}");
    }
    assert!(app.ui.rail_panels.is_empty());
    app.session.prefs.edit(|p| p.workspace_locked = true);
    assert!(app.run("window.panel.toRail", json!({"panel": "layers"})).is_err());
}

#[test]
fn dragging_a_floating_window_onto_the_icon_bar_docks_it_there_and_its_icon_opens_it() {
    let shots = std::env::var_os("PHOTOCRAFT_FLOAT_SHOTS").map(std::path::PathBuf::from);
    let mut app = app();
    app.run("window.panel.navigator", json!({})).unwrap();
    let mut ready = false;
    let mut builder = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0);
    if shots.is_some() {
        builder = builder.wgpu();
    }
    let mut h = builder.build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                app.ui.theme = crate::theme::ThemeKind::ProMedium;
                ready = true;
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    h.run_steps(4);
    let shot = |h: &mut Harness<'static, PhotocraftApp>, name: &str| {
        if let Some(dir) = &shots {
            std::fs::create_dir_all(dir).unwrap();
            h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
        }
    };
    shot(&mut h, "bar-1-floating");
    let rail = h.ctx.data(|d| d.get_temp::<egui::Rect>(rail_rect_id())).expect("bar drawn");
    let tab = h.ctx.read_response(egui::Id::new("photocraft-panel-docking").with(("tab", &"navigator".to_string()))).unwrap().rect;
    drag(&mut h, tab.center(), egui::pos2(rail.center().x, rail.bottom() - 60.0));
    h.run_steps(3);
    assert_eq!(h.state().ui.rail_panels, ["navigator"], "docked to the bar");
    assert!(floating_rect(h.state(), "navigator").is_none());
    shot(&mut h, "bar-2-docked");
    // Its icon opens the panel beside the bar.
    let icon = h.get_by_label("Navigator").rect().center();
    h.event(Event::PointerMoved(icon));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: icon, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.event(Event::PointerButton { pos: icon, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().ui.rail_open.as_deref(), Some("navigator"));
    assert!(navigator_left_of_dock(&h), "the Navigator panel in its flyout");
    shot(&mut h, "bar-3-flyout");
}

#[test]
fn undocking_from_the_icon_bar_makes_a_floating_window_by_default() {
    let mut app = app();
    app.run("window.panel.toRail", json!({"panel": "layers"})).unwrap();
    app.run("window.panel.fromRail", json!({"panel": "layers", "x": 120, "y": 140, "width": 320, "height": 420})).unwrap();
    assert!(app.ui.rail_panels.is_empty());
    assert_eq!(floating_rect(&app, "layers"), Some([120.0, 140.0, 320.0, 420.0]));
}

fn bar_harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                app.ui.theme = crate::theme::ThemeKind::ProMedium;
                ready = true;
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    h.run_steps(4);
    h
}

#[test]
fn dragging_an_icon_off_the_bar_undocks_its_panel_into_a_window_where_it_lands() {
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    app.run("window.panel.navigator", json!({})).unwrap();
    app.run("window.panel.toRail", json!({"panel": "navigator"})).unwrap();
    let mut h = bar_harness(app);
    let icon = h.get_by_label("Navigator").rect().center();
    // Let go on the bar: nothing changes.
    drag(&mut h, icon, icon + vec2(0.0, 6.0));
    assert_eq!(h.state().ui.rail_panels, ["navigator"]);
    drag(&mut h, icon, egui::pos2(400.0, 300.0));
    assert!(h.state().ui.rail_panels.is_empty(), "off the bar");
    let rect = floating_rect(h.state(), "navigator").expect("a floating window");
    assert!((rect[0] - 376.0).abs() < 2.0 && (rect[1] - 288.0).abs() < 2.0, "where it was dropped: {rect:?}");
}

#[test]
fn dragging_an_open_flyout_by_its_title_tears_it_off_into_a_window() {
    let mut app = app();
    app.run("window.panel.navigator", json!({})).unwrap();
    app.run("window.panel.toRail", json!({"panel": "navigator"})).unwrap();
    app.ui.rail_open = Some("navigator".into());
    let mut h = bar_harness(app);
    assert!(navigator_left_of_dock(&h), "flyout open");
    let title = h.ctx.read_response(egui::Id::new("photocraft-rail-flyout-grip")).expect("title bar").rect;
    let from = egui::pos2(title.left() + 40.0, title.center().y);
    drag(&mut h, from, egui::pos2(200.0, 200.0));
    assert!(h.state().ui.rail_panels.is_empty());
    assert!(floating_rect(h.state(), "navigator").is_some(), "torn off into a floating window");
    assert_eq!(h.state().ui.rail_open, None);
}

#[test]
fn icon_bar_icons_move_by_command_and_reject_bad_input() {
    let mut app = app();
    app.run("window.panel.railMove", json!({"group": "history", "index": 0})).unwrap();
    assert_eq!(rail_group_order(&app), [Group::History, Group::Properties, Group::Navigator, Group::Color, Group::Layers, Group::Brushes]);
    for panel in ["navigator", "histogram", "info"] {
        app.run("window.panel.toRail", json!({"panel": panel})).unwrap();
    }
    app.run("window.panel.railMove", json!({"panel": "info", "index": 0})).unwrap();
    assert_eq!(app.ui.rail_panels, ["info", "navigator", "histogram"]);
    for params in [
        json!({"index": 0}),
        json!({"group": "character", "index": 0}),
        json!({"group": "history", "index": 9}),
        json!({"panel": "layers", "index": 0}),
        json!({"panel": "info", "index": 3}),
        json!({"panel": "info", "index": -1}),
    ] {
        assert!(app.run("window.panel.railMove", params.clone()).is_err(), "{params}");
    }
    assert_eq!(app.ui.rail_panels, ["info", "navigator", "histogram"]);
    // Saved with the workspace; Reset Workspace brings back the default order.
    let restored: crate::state::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert_eq!(restored.rail_groups, app.ui.rail_groups);
}

#[test]
fn dragging_icons_along_the_bar_rearranges_them() {
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    for panel in ["navigator", "histogram", "info"] {
        app.run("window.panel.toRail", json!({"panel": panel})).unwrap();
    }
    let mut h = bar_harness(app);
    let rail = h.ctx.data(|d| d.get_temp::<egui::Rect>(rail_rect_id())).unwrap();
    let icon = |h: &Harness<'static, PhotocraftApp>, label: &str| h.query_all_by_label(label).map(|n| n.rect()).find(|r| rail.contains(r.center())).unwrap();
    // A built-in button: History dragged above Properties.
    let history = icon(&h, "History");
    let properties = icon(&h, "Properties");
    drag(&mut h, history.center(), egui::pos2(properties.center().x, properties.top() + 2.0));
    h.run_steps(2);
    assert_eq!(rail_group_order(h.state()).first(), Some(&Group::History));
    // A docked panel: Info dragged above the first docked icon. (The Navigator group's own
    // button stepped aside, so "Navigator" is the docked panel's icon.)
    let info = icon(&h, "Info");
    let navigator = icon(&h, "Navigator");
    drag(&mut h, info.center(), egui::pos2(navigator.center().x, navigator.top() + 2.0));
    h.run_steps(2);
    assert_eq!(h.state().ui.rail_panels, ["info", "navigator", "histogram"]);
    assert!(floating_rect(h.state(), "info").is_none(), "moved along the bar, not undocked");
}



/// The Navigator's body (its "No document" label, with no document open) drawn left of the icon
/// bar: in a flyout or a separate window, not in the dock.
fn navigator_left_of_dock(h: &Harness<'static, PhotocraftApp>) -> bool {
    let Some(rail) = h.ctx.data(|d| d.get_temp::<egui::Rect>(rail_rect_id())) else { return false };
    h.query_all_by_label("No document").any(|node| node.rect().center().x < rail.left())
}

/// #7896: the dock's widest is the window less what the toolbar and the canvas keep, never
/// below the old 520 points, so a wide window allows a two-column dock.
#[test]
fn the_dock_can_be_as_wide_as_the_window_allows() {
    for (window, max) in [(2560.0, 2160.0), (1280.0, 880.0), (800.0, 520.0), (f32::NAN, 520.0)] {
        let range = crate::panels::dock_width_range(window);
        assert_eq!((*range.start(), *range.end()), (crate::panels::DOCK_MIN_WIDTH, max), "{window}");
    }
}

#[test]
fn rejected_panel_detach_preserves_dock_and_return_positions() {
    for customized in [false, true] {
        let mut app = app();
        if customized {
            app.run("window.panel.move", json!({"panel":"navigator", "anchor":"layers", "zone":"top"})).unwrap();
        }
        let before = serde_json::to_value(&app.ui).unwrap();
        let result = app.run("window.panel.detach", json!({"panel":"navigator", "x":0, "y":0, "width":MAX_WINDOW_SIZE + 1.0, "height":300}));
        assert!(result.is_err());
        assert_eq!(serde_json::to_value(&app.ui).unwrap(), before, "a rejected detach must not float the panel");
    }
}

#[test]
fn rejected_panel_undock_preserves_the_open_rail_flyout() {
    let mut app = app();
    app.run("window.panel.toRail", json!({"panel":"navigator"})).unwrap();
    app.run("window.panel.toRail", json!({"panel":"histogram"})).unwrap();
    app.ui.rail_open = Some("navigator".into());
    let before = serde_json::to_value(&app.ui).unwrap();
    for params in [json!({"panel":"navigator", "to":"float", "width":-1}), json!({"panel":"navigator", "to":"float", "x":"invalid"})] {
        assert!(app.run("window.panel.fromRail", params).is_err());
        assert_eq!(app.ui.rail_open.as_deref(), Some("navigator"), "a failed move must not dismiss the flyout");
        assert_eq!(serde_json::to_value(&app.ui).unwrap(), before);
        assert_eq!(app.ui.rail_panels, ["navigator", "histogram"]);
    }
}
